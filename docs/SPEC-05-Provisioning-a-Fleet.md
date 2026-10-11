# SPEC-05: Provisioning a Fleet — from an empty workstation to a booting fleet server

* **Status:** **Normative and PARTLY BUILT.** Ruled by the maintainer
  2026-09-07 after a bring-up he described in the harshest terms as a failure
  from start to finish (`.rulings/` R-302); nothing in it was invented by an
  agent. **§2 (`rivet-reset`, and so
  §4.1), §2.6, §2.7, §4.0, §4.5, §4.6, §4.7, §4.9, §4.10, §4.11, §4.12, all of §5,
  §5.0, §5.1 and the three defects §9 and §11 used to list as §9.1, §9.2,
  §9.3 and §11.2 are built** — see the History at the bottom. §4.2, §4.3 and
  §4.4 are still written against behaviour this tree lacks, and §7's
  acceptance has **not** been measured on a machine.
  **`cargo rivet provision` was rebuilt 2026-09-13, then reverted the same
  day** (`.rulings/` R-174, R-175, R-176 Q11): R-174 made it cross-build,
  sign, publish and pin a role's own `rivet-fleetd`/`rivet-proxyd`, and build
  a whole card image, on every run; R-175 ruled that provisioning selects
  prebuilt, signed artifacts from the fleet repository and never builds or
  signs anything, and R-176 Q11 ruled the build-per-run code reverted while
  keeping the fixes made alongside it. **Then, the same day, R-178 P3/P4/Q7/Q10
  built two more pieces of R-176's model**: the interview
  (`cmd/rivet-provision`) now lists a role's required application versions
  against the fleet repository and lets the operator pick, newest
  preselected, writing `[carry.<product>] version = "…"` into the document
  itself (P3); and `cargo rivet provision` — not the interview — now commits
  the finished document to the fleet repository and enrolls a server or
  member into the operator's register, and does either ONLY after
  `write-image` has actually written and verified a card, never at the
  moment the interview's own `w` key saves the document locally (P4/Q10),
  with the duplicate-enrollment check running BEFORE that write (Q7/Q10).
  §4.5–4.12 below describes the command as it now stands: preflight, the
  interview (which itself now selects application versions), a
  staging-bundle check, an enrollment-duplicate check, the write, then
  recording — an image still comes from `--image PATH` or `write-image`'s
  own menu, never from anything this command builds. Selecting a prebuilt
  *image* from the fleet repository (as opposed to the application versions
  a role carries, which IS built) is still R-176's later, unbuilt work.
  **The same day, `.rulings/` R-180 added the redirect the Q7/Q10 duplicate
  refusal names**: `--existing HOSTNAME` writes an already-provisioned
  machine's card again without provisioning it again, taking its recorded
  document onto a selected image, with no interview, no re-enrollment and no
  second commit — and enrollment now records a static address the same way
  `cmd/rivet-fleet`'s own `add document` would. See the History for what
  changed and why. §7's acceptance is still **not** measured on a machine;
  that gap is unchanged by any of this.
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Artifact kind:** `rivet-provision`, `rivet-fleet`, `cargo-rivet`
* **Decides:** nothing. `ADR-004`, `ADR-025`, `ADR-034` and `ADR-019` are the
  decisions; this is what the tools must do about them.
* **Builds on:** `ADR-004` (provisioning documents), `ADR-019` (fleet
  enrollment), `ADR-025` (fleet identity), `ADR-034` (the fleet register),
  `ADR-028` (trust), `ADR-026` (the build loop)
* **Evidence:** `docs/design-candidates/Fleet-Bring-Up-Findings.md` — the run
  that produced this document, and
  `docs/design-candidates/Provisioning-Tool-Completion.md` §6, the requirement
  as the maintainer stated it

---

## 1. Scope

**This document covers one journey: an empty workstation to a fleet server that
boots already serving its fleet.** It stops where `SPEC-01` and the playbook take
over — building the project, packaging applications, administering a running
machine.

**It is written as an acceptance test, not as a design.** Every requirement is
phrased so that it can be failed. How the tools are structured internally is not
this document's business.

### 1.1 The one sentence this document exists to satisfy

The maintainer's expectation: when the device boots up, `fleetd` comes up running
just like `mcpd` does. *(`.rulings/` R-303.)*

**`rivet-fleetd` MUST reach `started` on the machine's first boot, from the
card, with its own register present and its own credentials in place.** Not
installed-and-idle. Not failed pending a credential somebody hand-installs
afterwards. **Running.**

On 2026-09-07 reaching that state took, after the card had booted: three secret
installations under names no tool would guess, two directory creations, three
file writes, five ownership changes, and two service restarts — every one of
them typed by an agent over the control plane. **That is the failure this
specification ends.**

**IT CANNOT BE SATISFIED BY A CARD THIS TREE CAN BUILD, and `ADR-050` is what
changes that.** `rivet-mcpd` is in the system image; `rivet-fleetd` is an
application, and `cmd/rivet-image` has no way to carry one. So a fresh card
boots with no `rivet-fleetd` to start — `ADR-049` removed every piece of
hand-work *around* the application while leaving the application itself absent.

`ADR-050` rules that the card carries signed payloads chosen by the machine's
role, installed by `rivet-deploy` on the first convergence. **The carrying and
the selection are both built now** (`ADR-050` §11.1, §4.8) — `cmd/rivet-image
--fleet-repository` reads the composed document's own enabled application
services and resolves each by digest, so a fresh card asked for a server role
carries `rivet-fleetd` without an operator naming it by hand. **`cargo rivet
provision` carries the identical set onto a PREBUILT, already-selected
image** (`ADR-054` §3–§4, Wave C3), rather than requiring one built fresh for
each computer — `.rulings/` R-175's whole argument, closed. **Whether this
happens on a boot is a question only a board can answer**: §7 is the honest
account of that, and hardware is what settles it, not more workstation
tooling.

### 1.2 The grading rule

The maintainer's rule: every time he could not do something and the agent had to
do it for him was, frankly, the system failing. *(`.rulings/` R-301.)*

**A step that requires an agent has failed, whatever it produces.** A step that
requires reading Rust source has failed. A step that requires hand-editing TOML
has failed. This rule governs every ambiguity below: where this document is
silent, ask whether the maintainer alone could do the thing.

---

## 2. `rivet-reset` — returning to a known-empty state — **BUILT**

**A fleet is minted more than once in a project's life, and the second minting
must not inherit the first's leftovers.** On 2026-09-07 a device record written
in August appeared in a fleet minted that afternoon, because it was lying in a
directory something else was reading.

`cmd/rivet-reset` is the program. It is its own binary and not a verb of
`rivet-provision`: that tool mints and this one destroys, and a person
rehearsing the minting ceremony must not be one keystroke from the program that
removes what a previous ceremony produced.

### 2.1 What it MUST remove

| | |
| :--- | :--- |
| the certificate authority | `~/.rivet/ca` — root, leaves, `suffix`, the PKCS#12 |
| the register | every device record and the fleet file |
| machine documents | composed provisioning documents held on the workstation |
| **the browser** | the authority's root **and every leaf it issued**, from the Windows certificate stores |

**The browser is not optional and not a follow-up chore.** Leaving a retired
root in the store is what made a browser offer four certificates of which none
could authenticate — and because a re-minted authority may carry the **same
subject name** as the one it replaces, they are indistinguishable to the person
choosing between them.

### 2.2 What it MUST NOT touch

* The **signing seeds** — `~/.rivet/keys/rivet-system.seed` and
  `rivet-app.seed`. They are not fleet identity, every board's trust store
  carries their public halves, and destroying them invalidates every payload
  ever signed. **`rivet-reset` MUST refuse to remove these and MUST say so.**
* Any **SSH key**, under any circumstance.
* Any **board**. Reset is a workstation operation.

### 2.3 How it MUST behave

* It **MUST name every path and every certificate it would remove, and require
  confirmation**, before removing anything.
* It **MUST be idempotent** — running it on an already-clean workstation is a
  report, not an error.
* It **MUST leave the workstation in the state a first-ever run would find**, so
  that §4 behaves identically for a new user and for a re-mint.

**Certificates are matched on the authority's KEY and never on its name**, which
is the requirement §2.1's second paragraph implies and the one thing here that
could be got quietly wrong. A certificate belongs to the authority being reset
if and only if that authority's public key verifies its signature. The authority
key identifier is corroboration and decides nothing: it is a hint copied out of
the issuer at minting time, it is optional, and anything that can write a
certificate can write any value into it.

**A certificate carrying the authority's subject name that its key does NOT
verify is a third answer, not a fourth-and-a-half.** It is the *other*
`home-fleet` — the retired one, or a rehearsal's throwaway — and it is named in
the report and **left alone** unless `--include-retired` is given. That is what
turns *"you have four certificates"* into *"three of these are from an authority
whose key is not here"*.

**The seed refusal stops the whole run**, not just the file. A reset that
skipped the dangerous file and removed the rest would leave a workstation in a
state nobody described, and the operator would find out from what still worked
rather than from a message.

### 2.4 It is still what clears a store somebody has already dirtied

**Rehearsal itself no longer needs it.** `rivet-provision ca --no-install`
writes `browser.pfx` and opens no store, and both rehearsal harnesses pass it
and assert that nothing was filed — so practice is free.

What `rivet-reset` is still for is the **retired authority and the leaves it
issued**, which are already in the store and which no flag removes. A browser
offering four certificates of which none can authenticate is the state §2.1
describes, and only removal fixes it.

### 2.5 What is proven, and the one thing that is NOT

**Removal from a Windows certificate store is UNVERIFIED.** Nobody has run it,
and it must not be described as working until the maintainer has.

The reason is deliberate: the only store available to test against is the
maintainer's own, it holds certificates belonging to other software, and
repairing it by hand cost him an afternoon on 2026-09-07. **An agent must not
write to it**, and there is no sandboxed store that would prove anything about
the real one.

What IS proven, and by what:

| | |
| :--- | :--- |
| the files — what goes, what stays, the seed refusal, idempotency | `tools/provision/rehearse-reset.sh`, against a scratch `~/.rivet` |
| **the matcher, against two authorities sharing one subject name** | the same script: two throwaway authorities are minted `CN=home-fleet`, every certificate of both is put to `--verdict`, and the six answers split on the key while every name in them agrees |
| reading the three stores | `cmd/rivet-reset/src/store.rs`'s own unit test, read-only |
| the matcher against the **real** store's contents | `rivet-reset --root <a copy of root.crt> --dry-run`, which selected 3 of 77 and left the Apple device certificate and four `trust_<guid>` entries alone |

**What closes the gap is one command, and it is the maintainer's to run:**

    rivet-reset --include-retired

Expect it to name the current root in `Root` and `CA`, the operator's own
`CN=Joel Lindell` leaf in `My`, the retired `CN=home-fleet` root, and any
rehearsal leftover carrying that name — and to leave everything else. Afterwards
`Get-ChildItem Cert:\CurrentUser\My` should hold no certificate issued by
`CN=home-fleet`, and a restarted browser should offer none.

### 2.6 It looks for a backup before it destroys anything — **BUILT**

**Nothing checked this before 2026-09-09.** §2.1's table removes `root.key`
file by file — `remove_file` on each, then `remove_dir` — and until this
section was built, nothing asked whether a copy existed anywhere. Losing it
orphans every certificate the current fleet's machines carry, with no way to
issue another.

`--backup <directory>` says where to look. `rivet-reset` compares the
authority's own `root.crt` against `root.crt` at that path and one level of
its immediate subdirectories — **byte for byte**, never by name and never by
opening a key. That is the same file `rivet-provision ca backup <directory>`
writes (§4.0), so an exact match is what a real backup produces and nothing
else does.

**Found or not, the report says so**, in the report this program already
prints before anything is confirmed (§2.3).

**If none is found, one extra word is required to proceed:
`--proceed-without-backup`.** Running with no `--backup` at all counts as
none found — there is no location to check. This is not a wall: the
maintainer's response to people who skip the instructions and lose their
keys is a shrug (`.rulings/` R-316) — the point is that the tool says it once, not that it
argues. `--dry-run` prints the same finding and removes nothing regardless of
whether the word was given.

**If the authority is genuinely gone and a backup exists**,
`rivet-provision ca restore <directory>` is the recovery — see §4.0.

### 2.7 `--keep-authority` — a new fleet under the same authority — **BUILT**

**A fleet is sometimes rebuilt without its authority changing.** Then the
authority's root and the operator's browser certificate stay installed in the
Windows stores, and nothing about the browser is redone; every record of the
old fleet's machines goes.

    rivet-reset --keep-authority --dry-run
    rivet-reset --keep-authority --confirm

**What it MUST remove:**

| | where | how |
| :--- | :--- | :--- |
| the operator register | `rivet-fleet`'s register directory (`--register`, resolved as §2.1's is) — device records, the fleet file, and the credential store beside them | file by file, as the ordinary reset does |
| the fleet repository's machine records | `register/` and `documents/` under `~/.rivet/fleet` (`--fleet`) | `git rm`, and **one commit** whose subject names `rivet-reset --keep-authority` |
| provisioning notebooks | `~/.rivet/provision` (`--provision`) — composed documents, staging bundles, answer notebooks | file by file |
| the Armada's Fleets | `~/.rivet/armada/fleets` (`--armada`) | file by file |

**What it MUST NOT touch**, beyond §2.2's list, and the report MUST name each:
the authority directory; the Windows certificate stores, which this mode never
opens; the fleet repository's `manifest.toml` and `objects/`, which are the
published software a fresh fleet installs as the old one did; the Armada's own
`armada.toml`, which records where backups go and how many to keep and so
describes the operator rather than a fleet; and every identity under
`~/.rivet/identity`.

**The fleet repository is cleared through git**, because it exists for
attribution (`ADR-052` §5): a removal behind git's back would leave a dirty
tree that the next provisioning run commits through, attributing the old
fleet's disappearance to a new machine's document. **A repository with
uncommitted or untracked changes MUST be refused by name before anything
anywhere is removed** — a reset does not commit somebody else's work, and a
refusal after the register had gone would leave a workstation half-reset.

**It MUST also refuse** a run whose plan reaches inside anything it keeps (a
`--provision` that names the authority directory, say), and the options that
only concern removing the authority — `--include-retired`, `--root`,
`--verdict`. `--fleet` and `--armada` are refused without `--keep-authority`,
because the ordinary reset clears neither. **`--backup` is not required**:
the authority is not removed, so there is nothing a backup must cover.

§2.3 applies unchanged: every path listed before anything is removed, with a
count per location; confirmation required; an already-empty fleet is a
report, not an error.

**Proven**, by `cmd/rivet-reset/tests/keep_authority.rs` running the built
binary against a scratch `HOME` whose registers, projection, documents and
Armada are laid out by `rivetos-register` and `rivetos-fleet-repo`: a dry run
changes nothing and reports the plan; `--confirm` empties the four locations,
leaves the authority, keys, identities, manifest, objects and `armada.toml`
byte for byte, adds exactly one commit and leaves the tree clean; a modified
or untracked file in the repository is refused with every file still in place.
**Not proven:** a run against a real workstation's own `~/.rivet` and
`%APPDATA%\rivet-fleet`, which is the maintainer's to make.

---

## 3. The operator script

**"Script" here is theatrical, not executable.** Ruled 2026-09-07: what is meant
is a list of instructions for a human to follow, and not a shell script.
*(`.rulings/` R-302.)*

**A shell script that performs §4 unattended does not satisfy this document.**
It hides which step failed, and it is the agent doing the work again. What is
required is an ordered list of steps the maintainer performs, where the **tool**
does the work at each one.

### 3.1 The test each line MUST pass

1. **He can perform it with no agent present.**
2. **He can tell from what comes back whether it worked** — every step has an
   observable outcome, stated in the line itself.
3. **A failure names the step**, not a stack trace and not a later symptom.

### 3.2 Where it lives

In `docs/`, beside this document, generated from the tool where possible so it
cannot drift from what the tool does. `rivet-fleet export card-plan` is the
existing attempt at this shape and **it currently emits three wrong
instructions** (§9.2), which is the argument for generating it from verified
behaviour rather than from a template somebody maintains by hand.

---

## 4. The twelve steps

**These are the maintainer's, in his order.** Each carries what the tool MUST
do and the failure it exists to prevent.

### 4.0 Back up the authority — **BUILT**

**Numbered zero, and not renumbered into the twelve, because the twelve are
already cited by number from outside this document** (`ADR-004` §4,
`ADR-049`, `docs/PLAN-Provisioning-2026-09-07.md`) and rule 6 (`AGENTS.md`
§2) makes a heading immortal once anything links to it. This step exists and
goes **before step 1**, because step 1 destroys the only copy of what this
step exists to save.

    rivet-provision ca backup <directory>

**`<directory>` is REQUIRED and there is no default.** Ruled by the
maintainer 2026-09-09: a default location for storing secret keys is an
absurd statement. `cargo rivet backup`'s `--into` and Fleet Monitor's
`backups.folder` are the same rule for the same reason — a tool that invents
a destination is one whose output is somewhere the operator did not choose,
and they find out when they need it.

**Observable:** every file copied, named, with its size — the evidence, not
somebody's memory of having run the command. `root.crt`, `root.key`,
`browser.crt`, `browser.key`, `browser.pfx`, `suffix`, and a
`server-<name>.crt`/`.key` pair per fleet server this authority has issued
for — whatever is actually in `~/.rivet/ca`, never a hard-coded list.

**It refuses to overwrite an existing backup silently.** A file already at
`<directory>` is named with its size and age, and the run stops; `--replace`
overwrites it. A backup that quietly replaced last week's good copy with
today's broken one is the failure this step exists to prevent.

`rivet-provision ca restore <directory>` is the inverse, and it is the
inverse of `rivet-reset` — not of a plain copy. See §2.6.

### 4.1 Reset the CA, including the browser — **BUILT**

Per §2: `rivet-reset --include-retired`. **Observable:** a list of what was
removed, and a store containing no certificate issued by the retired authority.

**The second half of that observable is what §2.5 says is unverified.** Read the
report before confirming; it names every certificate and says why each one is
going or staying.

**Rebuilding a fleet under the SAME authority is the other route:**
`rivet-reset --keep-authority` (§2.7), after which §4.2 and §4.3 have nothing
to do — the authority and its installed root are already in place.

### 4.2 Mint the new CA

The authority, its suffixes, and the operator's own certificate.

**A workstation with no authority is first asked whether it is starting a new
fleet or joining an existing one, and nothing is minted until it answers**
(`ADR-063` §8). The interview's first screen on such a workstation is a
numbered question — `1` a new fleet, `2` join an existing one, `x` leave —
with no default, because a bare Return is not a decision to mint a fleet's
trust. **New** goes on to the fleet name, the suffixes and the operator's name,
and mints. **Join** mints nothing, says what a joining workstation carries
across today — `~/.rivet` and `rivet-fleet`'s register from a workstation
already in the fleet (`ADR-062` §5.6) — and stops. A workstation that already
holds an authority is never asked.

**With no terminal to ask at, the interview refuses rather than mints** unless
`rivet-provision --new-fleet` says in advance that this is a new fleet. `cargo
rivet provision` refuses an interview with no terminal before it gets that far.

**Observable:** on a workstation with no `~/.rivet/ca/root.crt`, the first
screen is `new fleet, or join one?`; after `2` or `x` nothing has been
written under `~/.rivet/ca`.

**It MUST refuse, or require explicit override, when the new authority would
carry the same subject name as one it is replacing** — unless §4.1 has already
removed that one. Two authorities named `CN=home-fleet` is what made a
certificate picker unusable.

### 4.3 Tell him to install `root.crt` — and let him do it

**This step is deliberately his.** Trusting a root certificate is a decision a
person takes, and a tool that installs one silently has taken it for them.

**Observable:** the tool names the file, says which store, and says what he
should see afterwards.

### 4.4 Install his browser certificate FOR him

**The tool does this one.** He clicks OK on the Windows prompt and nothing else.

**It MUST verify the private key is present after import and say so.** On
2026-09-07 the correct certificate sat in the Personal store with no private
key: browsers silently decline to offer such a certificate, so the symptom is
*"none of my certificates work"* with the right one invisible among them.

**`certutil -importpfx` reports success while leaving a key unattached when a
copy of the certificate is already in the store**, so the check is required and
a return code is not sufficient evidence.

### 4.5 Provision a new fleet server, correctly — **BUILT**

**"Correctly" means the composed document carries what the role needs** — §5.

**Observable:** the document, on screen, before anything is written.

The menu asks nine questions, and a tenth — the nameservers — for a static
address, and composes the role's own defaults from the Mode and Fleet answers
— §5 and §5.0, and `ADR-004` §6.2 is the decision. **Under the
document it prints what the role decided and what each line costs if the rest of
the card does not match it**: that a reachable listener requires a TLS identity
on the card, which `cargo rivet provision` puts there, and that nothing here can
check whether a kiosk's application will be installed. That warning is not
printed for an auto-run the system image itself carries — a name on
`rivet_core::service::IMAGE_SERVICE_NAMES`, such as `rivet-launcher`.

**It never tells a person to build a card** (`ADR-054` §2). Run by `cargo
rivet provision`, which sets `RIVET_PROVISION_ORCHESTRATED` on it, the
interview says only that the orchestrator puts the identity on the card, and
names no next step after `w`: the same run writes the card. Run directly, `w`
ends by naming `cargo rivet provision --document <document> --identity
<directory>`, the command that writes a card from what it just saved.

**Built 2026-09-13** (`.rulings/` R-178 P3, R-176 Q6): before any of that is
printed, the interview also picks a version for every product the role must
carry (`rivet_config::Document::carried_services` — `rivet-fleetd` for a
server, nothing for a member, a kiosk's own auto-run application; the proxy is
part of the image, `ADR-060`). For each one it lists what the fleet repository has
published (`cmd/rivet-provision/src/versions.rs`, a third reader of
`manifest.toml` beside `cmd/cargo-rivet/src/fleet.rs` and
`cmd/rivet-image/src/fleet_payloads.rs`), newest first, and the operator
picks — a bare Return takes the newest, never chosen silently. The choice is
written into the document as `[carry.<product>] version = "…"`
(`rivet_config::carry`) before `w` ever composes the printed document. **A
required product with no published version is refused right here, by name,
naming `cargo rivet fleet publish` — nothing is minted, printed or written
for a card that could not carry what its own role needs.** The repository
read from is `RIVET_FLEET_REPOSITORY`, set by `cargo rivet provision`'s
orchestrator to the same root it will commit into (§4.5–4.12), or —
run standalone, with no orchestrator — the same default every other fleet
tool resolves.

**The Identity question: one identity per machine** — built 2026-09-14
(`.rulings/` R-189 D2). The ninth field names the directory holding this
machine's `device.seed`, `device.crt` and `peers`. It MUST behave as follows:

* **Its default is `~/.rivet/identity/<hostname>`** (under `RIVET_AT` when set),
  and it follows the Hostname answer until the operator types an Identity of
  their own. A resumed notebook keeps the Identity it saved.
* **Editing fields writes nothing.** Quitting leaves no identity behind.
* **At `w`, before a version is asked or a file written**, a directory that
  holds an identity is checked against the register named by
  `RIVET_FLEET_REGISTER`. If another hostname's record carries its pin, `w` is
  refused, and the menu stays open:

  ```
  refused: NOTHING WAS WRITTEN. The identity given for <hostname> is already "<record hostname>"'s (register id <id>, in <register>): its pin is on that record. Two machines holding one private key answer for each other. Pick or mint another identity directory: the interview mints a fresh identity at `w` in a directory that holds none.
  ```

  Run without `cargo rivet provision`, no register is named, and `w` says it
  did not check. A record under the SAME hostname is left to §4.5–4.12 step 5,
  which names `--existing`.
* **A directory holding neither `device.seed` nor `device.crt` is minted at
  `w`**, after the document is written and read back, and before the staging
  bundle is composed: a fresh Ed25519 identity of the shape `cargo rivet
  identity new` mints, and a `peers` file naming the operator's key from
  `~/.rivet/identity/operator/device.crt`, which is what `cargo rivet setup`
  writes beside the board's. With no operator certificate there, nothing is
  minted and `w` fails by name. A directory holding one half is never minted
  over.
* **A repeated `w` for the same hostname reuses the `rivet-fleetd` operator
  credential its previous staging bundle carries**, so the identity's `peers`
  file gains that key once, however many times `w` runs.

**The Nameservers question: a static machine's resolver.** The tenth field. A
static machine has no lease to learn a resolver from, so a document that names
no nameservers is a machine whose every lookup fails inside the application
that made it. It MUST behave as follows:

* **It is asked only when Addressing is `static`.** Under `dhcp` it is not on
  the screen, typing its number is refused, and the document declares no
  `[network.resolver]`: the lease fills the resolver, and only while that table
  is silent (`rivet_config::resolver`). It is last, so hiding it moves no other
  field's number.
* **Its default is the Gateway answer**, and follows the gateway until the
  operator types an answer of their own. The prompt shows it, and a bare Return
  takes it:

  ```
    up to three IPv4 addresses, in the order asked, e.g. 192.168.1.1, 1.1.1.1
    Nameservers [192.168.1.1] >
  ```
* **It refuses at the prompt what the board's parser refuses, by name**: an
  entry that is not an IPv4 address, and more than three, because musl asks no
  more. It also refuses an empty list, which the parser would accept as a table
  that says nothing.
* **`w` composes it as `[network.resolver] nameservers = [...]`**, in the order
  typed, after `[network.eth0]`. The answer is saved in the notebook as every
  other is, and a resumed notebook that saved none follows its gateway.

**What is still absent is §5's second half** — the credentials and the register,
which are §4.9's and are not built.

### 4.6 ACTUALLY SAVE THE CONFIG — **BUILT**

**A menu key labelled as a write MUST write.** A person who typed it and saw
output scroll past reasonably believes a card was written.

`w` writes the composed document to `~/.rivet/provision/<host>.provisioning.toml`
and reports the path. **The suffix is not decoration:** the same directory holds
the `s` key's answers notebook under `<host>.toml`, and one name for two
artefacts would have had the two keys destroy each other's work.

**It names an existing file, its size, and asks, before replacing one.**

**Changed 2026-09-13** (`.rulings/` R-178 P4/Q10, R-176 Q10): `w` writes
this file, the `ADR-049` staging bundle beside it, and the breadcrumb
`cargo rivet provision`'s orchestrator reads — **and nothing else.** Until
this date it also committed the document into the fleet repository
(`rivetos_fleet_repo::record_document`) at this same moment, before any card
had been written, let alone verified — the ordering R-176 Q10 ruled against
in as many words: *"after the write, the final document with its pins is
committed to the repository."* That call moved to `cargo rivet provision`
itself, run only once `write-image` has actually written and verified a
card (§4.5–4.12). Run `rivet-provision` on its own, with no orchestrator to
call it afterward, `w` now commits nothing to the fleet repository at all —
only the local files above.

### 4.7 ACTUALLY CHECK IT WITH THE PARSER — **BUILT**

**The saved document MUST be read back and parsed by the same parser a board
uses**, and the result reported.

*"It looked right"* is not a check, and the schema is the only thing that knows.

`w` re-reads the file it just wrote, hands it to `rivet_config::Document::parse`
and prints the hostname, the interface count, the service count and the
effective control-plane listener. **A file that does not parse is reported as
such and the tool exits non-zero**, so a card is never built from one.

### 4.8 Create the image — **BUILT**

**Observable:** the image path and its digest.

**`cmd/rivet-image --fleet-repository DIR` resolves this step's own carrying by
itself**, closing the coupling this section used to leave to §4.9 and to the
operator — `PLAN-Feature-Set-2026-09-12` §4, `ADR-050` §5.1. Given `--document`
too, it reads back which application services the composed document already
enables (`rivet_config::Document::carried_services`, `ADR-050` §5.1's table
realised: a server's document carries `rivet-fleetd`, a kiosk's its auto-run
application; `rivet-proxyd` is enabled and never carried, `ADR-060`) and
resolves each by digest out of
the named fleet repository — the object `cargo rivet fleet publish` filed it
under — exactly as `--application PATH` has always carried a payload the
operator named by hand. `--application` and `--fleet-repository` are additive:
either naming a product the other already resolved is the existing
two-payloads-one-product refusal.

**It refuses rather than guessing which version to carry, unless the document
pins one.** A product with no `application` artifact in the manifest, or with
more than one, is refused by name — `AGENTS.md` §5 — naming what `cargo rivet
fleet publish` is for or every candidate found (with the newest suggested as
the key to set), rather than carrying whichever happens to sort last.
**Built 2026-09-13** (`.rulings/` R-159 R20, `ADR-050` §5.2): the document may
name which version to carry — `[carry.<product>] version = "…"`
(`rivet_config::carry`, read via `Document::carried_version`) — and when it
does, `resolve` carries exactly that version if the repository holds it, and
refuses by name if it does not, without regard to how many other versions are
also published.

**Built 2026-09-13** (`.rulings/` R-160 R26): a document composed through
`cmd/rivet-fleet` from the register can now carry a pin too, and not only a
hand-authored one. `rivetos_register::{Fleet, Device}::carry` and
`cmd/rivet-fleet/src/compose.rs::carry` compose it the way `service_memory`
already composes a memory cap — a device's own pin wins, a fleet-wide pin
fills the gap otherwise, and no role layer sits between them (`ADR-050` §5.2 —
no role knows a version). This step's own reading — `Document::carried_version`
— is unchanged either way; only which tool produced the document differs.

**Proven by unit test, on a scratch fleet repository**
(`cmd/rivet-image/src/fleet_payloads.rs`): a role's required product resolves
to its object by digest; an unpublished product is refused by name; two
published versions of one product are refused rather than guessed between, and
the newest is named as the key to pin; a pinned version present among several
resolves to exactly that one; a pinned version absent is refused by name
whether the product was ever published or not; a document enabling no
application service resolves nothing; a `library` of the same name is not a
candidate (`ADR-050` §4.5); a manifest naming bytes the object store lacks is
refused as inconsistent.

**Built 2026-09-13: the shell build recipe forwards it, so this is reachable
without a hand-typed `cargo run -p rivetos-image`.** Until now `cmd/rivet-image
--fleet-repository` existed and nothing called it — `tools/card/build.sh` took
no such input. `RIVET_FLEET_REPOSITORY`, given together with `RIVET_DOCUMENT`,
now passes it through on `RIVET_IDENTITY`'s own footing (a loud line either
way, refused without a document, on `cmd/rivet-image`'s own rule);
`tools/container/build-card.sh` mounts the directory read-only and forwards
the variable into the container, on the same footing again.

**Proven** by `tools/container/build-card.sh` against a fictional server-role
document (`hostname = "example-board"`, a TEST-NET-1 address, `fleet-role =
"server"`, `[services.rivet-fleetd] enabled = true`, `[carry.rivet-fleetd]
version` pinned) and a reused scratch fleet repository — under this project's
own scratchpad convention (`AGENTS.md` §8.3), never `~/.rivet/fleet` — already
holding three published `rivet-fleetd` artifacts, signed, from an earlier real
deploy. The build's own output named the carry:

    --- applications this card carries (ADR-050 §4) ---
      rivet-fleetd             application 0.1.86-g5d09122    2212766 bytes  -> app-rivet-fleetd.rvt

the exact digest, version and size the pin resolved to in the repository's
`manifest.toml`. **Also proven, the other side of the same code path**: the
identical document with `[services.rivet-proxyd] enabled = true` added was
refused by name — *"the role requires `rivet-proxyd` and the fleet repository
... has no `application` artifact of that name"* — because that repository has
never published one; a real server role carries both (`ADR-050` §5.1), but
proving that needs a second published product this fixture repository does not
have. The image stayed in a scratch `out/`; nothing was written to a device.

**Not proven, and unchanged by this**: that `tools/card/build.sh`'s own
`verify_carried_payloads` (`ADR-050` §4's boot-partition read-back) covers a
payload resolved through `--fleet-repository` — it only counts
`RIVET_APPLICATIONS`, so a payload carried this way is unverified at the FAT
level by this script, though `cmd/rivet-image` itself still VERIFIES it
against the trust store before writing it, exactly as it does for `--application`.
That gap is `tools/card/build.sh`'s own, not new. **Not proven: that a card
built this way boots to the payload installed.**
`ADR-050` §11.2 and `ADR-049` §9.2 both name the reason and it is unchanged by
this step existing — first boot's read of `--application`'s payloads rides the
same boot-partition mount `cmd/rivet-initd/src/provision.rs` uses for
`rivet.toml`, which calls `rivet_adapter_image::boot_partition_device` and
therefore needs `/proc/device-tree/chosen/bootloader/partition`. `ADR-026` §4
and `cmd/rivet-initd/src/confirm.rs`'s own header are explicit that `-M virt`
does not emulate that property at all, so this cannot be exercised under QEMU
— only a real card and a test board close it, the same three things §7 and
`ADR-050` §11.2 already name.

### 4.9 Inject the fleet register — **BUILT**

**The card MUST carry the fleet's register**, so the machine boots knowing its
own fleet.

**BUILT**, as one hop of a general mechanism rather than as a register-shaped
special case. `ADR-049` is the decision: `rivet-provision` composes a staging
bundle beside the document, `rivet-image --staging` writes it to the boot
partition, PID 1 moves it to `/state/staging` and destroys the card's copy, and
the **supervisor** materialises it into each application's data directory with
the uid it has just derived. The register goes in it as
`register/register.toml` beside `register/devices/<id>.toml`, rendered by
`rivetos_register` itself rather than by a second spelling of the layout, and
carrying exactly one record — the machine's own, with its pin read out of the
`device.crt` that will be on the same card.

Two things the requirement did not anticipate, both recorded in `ADR-049`:

* **The ownership hop is the whole of the new work.** *"Owned by that
  application, mode as the supervisor would create it"* is not something the
  card or PID 1 can do — the uid is derived at discovery — so delivery had to
  move into the supervisor, which holds both halves at once: the material the
  card carried, and the uid it has just derived.
* **The delivery usually happens later than first boot**, because no application
  is on a fresh card. The bundle waits under `/state/staging` and is delivered
  at the discovery pass that first sees the application installed, which is the
  same pass that creates the directory. That is `ADR-049` §5.1 and it bounds
  what §1.1 can claim today; see §7.

At minimum the fleet server's **own** record — `ADR-034` and
`cmd/rivet-fleetd/src/fleet.rs` both already say this is written onto the card
by `rivet-provision`, and **it is not.** Its absence is why a fleet server
serves an empty register and why there is no recovery once the card is built.

**It MUST be written in `rivetos_register`'s on-disk layout** —
`register.toml` beside a `devices/` directory — into `rivet-fleetd`'s data
directory, **owned by that application**, mode as the supervisor would create
it. A register the application cannot read is indistinguishable from an absent
one, and reports nothing.

**It MUST inject only the records the operator asked for.**

**The same bundle carries a fleet server's repository text**, under a
`fleet-repository/` prefix that cannot collide with `register/` — the fleet's
`manifest.toml`, `documents/` and `register/` projection, and **no object**,
since `.rulings/` R-182 (`ADR-054` §5). The objects are what overran the
bundle's cap on a real server.

### 4.5–4.12 as one command — **BUILT**

`cargo rivet provision`, plan §5.2 item B, `.rulings/` R-162, extended
2026-09-13 by `.rulings/` R-178 (P2–P4, Q7, Q10, and the platform note) and,
the same day, `.rulings/` R-180. Run with no `--document` and no `--existing
HOSTNAME`, it:

1. **Preflights credentials** — `cargo rivet setup --check`'s own survey —
   before anything else runs, so an operator is never told minutes into the
   interview that a key was missing all along. A missing fleet repository is
   offered here too (`cargo rivet fleet init`'s own work, declinable), the
   way a missing CA is already offered inside the interview.
2. **Selects a system image from the fleet repository — before the
   interview runs** (`.rulings/` R-178 P3, so a refusal here costs no
   typing). The platform comes from what the repository actually holds, never
   from this workstation's own cross-build target: one platform present is
   used and named on screen; more than one is asked before a single image is
   listed; a named `--platform TOKEN` with no image is refused, naming the
   platforms that do have one. That platform's `card` images are listed
   newest first, each checked against this build's own
   `rivet_config::SCHEMA_VERSION` — a mismatch is listed and marked with the
   reason rather than silently accepted, and cannot be selected (R-178 P2);
   if nothing for the platform is selectable, or the repository holds no
   images at all, this refuses by name — *"No images available for
   provisioning. Deploy a system image to your artifact repository prior to
   provisioning."* verbatim when the repository is simply empty. The newest
   selectable image is preselected; a terminal is asked only when more than
   one is selectable. `--image` may name this selection directly instead of
   the menu — a filesystem path (unchanged meaning) or a fleet-repository
   digest or an unambiguous prefix of one, checked against the platform and
   schema exactly like a menu pick. The selected image is verified against
   this workstation's own `keys/rivet-system.pub` and decompressed into a
   scratch working copy, which is removed once the run finishes, whichever
   way.
3. **Launches `cmd/rivet-provision`** (§4.5's interview — the same menu, the
   same CA-derived `upstream-name`, the same staged register, and, since
   R-178 P3, the same application-version questions), **found beside
   `cargo-rivet`'s own binary, so this command needs no source checkout and
   runs from any directory** (`.rulings/` R-191, R-192). Both binaries are
   installed — README §2's two `cargo install --path` lines — and the pair
   must be the same build: `cargo rivet provision` asks the sibling's
   `--version` and refuses a mismatch, naming both builds and the reinstall,
   because the two share the interview's files, its breadcrumb and the
   saved-answers notebook. With no sibling installed, a working directory
   that really is a Rivet checkout still gets `cargo run`, which is what
   makes an edit to `cmd/rivet-provision` take effect on the next run; with
   neither, the refusal names the install rather than a directory. It is
   launched with this process's own terminal and four variables on its
   environment:
   `RIVET_FLEET_REPOSITORY`, the repository this run will later commit
   into; `RIVET_FLEET_REGISTER`, the register step 5 checks, which the
   interview's `w` checks an existing identity against (§4.5);
   `RIVET_AT` (since 2026-09-14), this run's own credential root
   (`--at DIR`, or this host's own default), which `cmd/rivet-provision`
   reads before falling back to `HOME`/`USERPROFILE` for its CA, its
   saved-answers notebook, the document and `ADR-049` staging bundle it
   writes, and the breadcrumb this step reads back afterward; and
   `RIVET_PROVISION_ORCHESTRATED=1`, which keeps the interview from advising
   how to get its document onto a card (§4.5). **Before 2026-09-14,
   `RIVET_FLEET_REPOSITORY` was the only one set here**, so a `--at DIR`
   scratch run's interview still
   minted into, and offered to resume from, this HOST's real `~/.rivet/ca`
   and `~/.rivet/provision` — found building
   `cmd/cargo-rivet/tests/pty_interview.rs` (a real `resume which?` naming a
   real saved notebook, before a fix landed).
4. **Checks the composed document for a staging bundle.** A server or member
   document with no `ADR-049` staging bundle beside it is refused here, by
   name, before anything is written. The interactive interview already
   writes this bundle before `w` succeeds, so what this step actually catches
   is a hand-composed `--document PATH` that skipped the interview.
5. **Checks a server or member document for a duplicate enrollment** —
   built 2026-09-13 (R-178 Q7/Q10). `ADR-049` §8.1 derives the register
   identifier from the machine's own control-plane pin; if that identifier,
   or the document's own hostname, is already in the OPERATOR's register,
   this refuses by name, and nothing has been written. **The pin itself is
   compared too** (`.rulings/` R-189 D2): a pin already on a record under
   another hostname is refused with §4.5's sentence, naming that record's
   hostname and identifier, because a record enrolled by `rivet-fleet add
   device` carries a random identifier the pin does not derive. **Given `--at DIR`,
   the register consulted is `DIR/register`** (`crate::provision::register_root_for`,
   2026-09-14) — never `crate::device::default_register_dir`'s
   `ignition::config`/`%APPDATA%\rivet-fleet\register` answer, which is the
   OPERATOR'S REAL register and has no relationship to `--at` at all. Without
   `--at`, nothing changes: `default_register_dir`, the same one `cargo
   rivet run <device>` already resolves, is still what this checks against.
   The pin comes from the interview's own breadcrumb when one ran, or from
   `--identity PATH`/`--pin HEX` otherwise — the route `--document PATH`
   needs, since it runs no interview and produces no breadcrumb. A
   stand-alone document needs none of this.
6. **Calls `write-image`** (§4.10–§4.12) with the image step 2 selected.
   **Widened 2026-09-14, Wave C3 (`ADR-054` §3–§5):** before the write, the
   working copy's boot partitions also receive the `ADR-049` staging bundle,
   this card's own identity (`device.seed`, `device.crt`, `peers`), and one
   `app-<product>.rvt` per `[carry.<product>]` pin — the exact names and
   bytes a fresh `cmd/rivet-image` build would have placed for the same
   inputs, so first boot needs no change. Each carried payload is verified
   against this workstation's own `keys/rivet-app.pub` before it is placed,
   refusing by name rather than writing an unverified one (`ADR-050` §4.6);
   a fleet **server**'s document additionally carries the fleet repository's
   **text half only** — `manifest.toml`, `documents/`, `register/`, and no
   object (`.rulings/` R-182) — staged into `rivet-fleetd`'s own data
   directory, refused by name when it will not fit the staging cap
   (`ADR-054` §5). A file that will not fit a boot partition's own free space
   refuses by name before either partition is touched.
   **The device step (2026-09-14, R-174).** With a terminal, `cargo rivet
   provision` always proceeds to a device: `--device PATH` names one, and
   otherwise the removable devices are offered as a menu (§4.10), the plan
   is printed, and `Write it? [y/N]` is asked. Only `y` raises the
   administrator prompt (§4.11), then the write and the read-back. `confirm`
   means the question was already answered and is not asked. **`N`, a
   cancelled administrator prompt, or no removable device writes nothing
   and records nothing**; the run says so and names the command that writes
   the same card later without the interview — `cargo rivet provision
   --document <path> --identity <path> --image <digest> confirm`, with this
   run's own paths and digest filled in (and `--at DIR` when given). It never
   names the image's working copy, which is removed when the run exits.
   Without a terminal, an image is only prepared unless `--device PATH` or
   `confirm` is given.
7. **Records — built 2026-09-13 (R-178 P4/Q10), and ONLY here, ONLY after
   step 6 has actually written and verified a card** (an image merely
   prepared, a declined `Write it? [y/N]`, or a cancelled administrator
   prompt is not a finished card, R-174, and nothing is recorded for it).
   The document is committed to the fleet repository
   (`rivetos_fleet_repo::record_document`) and, for a server or member, the
   device planned in step 5 is enrolled into the operator's register and the
   register is projected into the fleet repository too
   (`rivetos_fleet_repo::record_register`) — both best-effort, on
   `rivetos_fleet_repo::Outcome::Refused`'s own footing: a card that was just
   written and verified must go on being reported as written whether or not
   either of these second copies could be made.
8. **Summarises — built 2026-09-14, Wave C3.** Only once step 6 has
   verified a card written to a device: hostname, role, the image
   (product, version, a digest prefix), every carried product and version,
   the register identifier step 7 enrolled (or that none was), the device
   path, and one line naming the still-unproven step and the exact command
   that closes it (`ADR-054` §7). It never claims the board boots — writing
   and reading a device back proves the bytes, not that a service started.
   **For a fleet server it then names what the server still needs once it
   answers** (2026-09-14, `.rulings/` R-182/R-184): its text, by `cargo rivet
   fleet push <host>` then `… confirm`, or — for a document declaring no
   `[repos.fleet]` — that the push will be refused and which tool mints the
   declaration; and then its objects, which are not on the card, by `cargo
   rivet fleet sync <host>` then `… confirm`, into the server's repository
   store over the control plane (`.rulings/` R-184 D1, `ADR-052` §7.4).

Pass `--document PATH` and step 3 above is skipped — image selection (step
2), the staging check, the enrollment check, the write and the recording all
still run, using `--identity PATH`/`--pin HEX` for step 5's pin since no
interview ran to supply one. **Without a terminal and no `--document` (and no
`--existing`), it refuses by name** rather than launching an interview
nothing can answer (decision 4, plan §2) — an agent on this route names its
image directly with `--image <digest-or-path>`, since a menu needs a terminal
step 2 does not have.

**`--existing HOSTNAME` — built 2026-09-13 (`.rulings/` R-180) — is the
redirect step 5's own refusal names, for writing an already-provisioned
machine's card again WITHOUT provisioning it again**: *"you can't provision
that which has been already provisioned"*, but a person redoing a card after
a failed boot, or rewriting `rivet-01`, still needs a way to do it. Given it,
steps 1's fleet-repository offer, 3 AND 5 are skipped — there is no
interview, nothing is minted, and step 7 never runs at all, on any route,
successful or not. What runs:

* Step 2, image selection, unchanged — `--existing` needs a card to write
  exactly as any other run does.
* The document already recorded for `HOSTNAME` — the fleet repository's
  `documents/<host>.provisioning.toml`, the one a VERIFIED write commits
  (step 7), never `~/.rivet/provision/`'s own copy, which can hold a
  composed document that was never carried through to one. `HOSTNAME` with
  no recorded document is refused by name.
* Step 4, the staging check, unchanged — against the recorded document,
  written back to the same local path `cmd/rivet-provision`'s own `w` key
  would have used, so the ORIGINAL staging bundle is found beside it on the
  workstation that ran the interview. On a different one, this refuses by
  name — R-180's own words, "locate it where provisioning left it, or refuse
  by name saying what is missing" — rather than a second lookup invented for
  this route.
* Step 6, the write, exactly as any other run. **The identity is the
  recorded machine's own**: `--identity PATH` when given, else
  `<at>/identity/<HOSTNAME>` when it holds a certificate (`.rulings/` R-189
  D2), else the board identity `write-image` falls back to, which every
  machine provisioned before per-machine identities was written with.

`--existing` and `--document PATH` may not both be given. Step 5's own
duplicate refusal now names this redirect command in full, `HOSTNAME` filled
in, whichever of the two duplicate shapes (the derived identifier, or the
hostname) it caught.

**R-180.1: a static address is recorded at enrollment.** Step 5/7's `Device`
now also carries `interfaces`, populated the way `cmd/rivet-fleet`'s own
`add document` loss-repair sets one — `method`/`address`/`gateway`, keyed by
interface name — for every interface the document addresses statically, so
the fleet can reach the machine straight away. A DHCP interface records
nothing, as it always has.

It calls `cmd/rivet-provision` and `write-image` each exactly once; neither
is re-implemented, and neither `record_document` nor `record_register` is a
copy — both are `rivet-fleet-repo`'s own, called from this process instead
of from `cmd/rivet-provision`. **Unit-tested**: the hand-off (image selection
running before the interview and identically on the `--existing` route; the
interview's document reaching the staging check and then the writer
unchanged; preflight running before the interview; a refusal from any stage
stopping the sequence there before the next stage or the write; the
enrollment check running before the write and the commit/enrollment running
only after a successful, device-targeting write; a scratch working copy
removed after the run finishes, success or failure) is proven with fakes — no
terminal, no subprocess, no disk beyond a scratch directory. The image
selector specifically is also proven against REAL scratch fleet repositories
holding real signed, zstd-compressed card artifacts (built the same way
`cargo rivet image`'s own end-to-end test builds one): the platform ask/use/
refuse decisions, a schema mismatch marked and unselectable, nothing
selectable refused by name, a digest or prefix resolved and checked like a
menu pick, and a literal `--image PATH` never touching the repository at
all — and, for the fleet recorder specifically, also against a REAL
`rivetos_fleet_repo`/`rivetos_register` pointed at scratch directories,
including a `--document PATH` scratch end-to-end that shows the document
committed and the register holding the enrolled record afterward, and that a
second run with the same pin is refused as a duplicate before anything is
written, naming the `--existing` redirect. The redirect route itself is
proven the same way — fakes showing no interview, no enrollment planning and
no commit on it while the write still happens, and an unknown hostname
refused by name — and `RealExistingDocument` is proven separately against
real scratch files: found and materialised locally, an unknown hostname
refused, and no fleet repository at all refused. A static address and a DHCP
one are both proven for `enrollment_device`. That `SubprocessInterview`
actually finds and runs a real `cmd/rivet-provision`, and drives its
interactive, version-selection menu, is R-178 P6's own item — proven with a
real ConPTY (Windows) or pty, `cmd/cargo-rivet/tests/pty_interview.rs`, three
`#[ignore]`d tests spawning the real `cargo rivet provision` binary and
typing real keystrokes: an empty fleet repository refused before any
question is asked, a schema-mismatched image marked and the run refused, and
the full happy path through to a written, read-back document with an
explicitly-typed (non-default) carried version. **The same harness also
proves the 2026-09-14 `RIVET_AT`/register fix** (this section's own item 3
and item 5): every run points `HOME`/`USERPROFILE` at a directory that must
never come to exist, and asserts exactly that afterward — proof, not
assumption, that `--at DIR` alone reaches the interview and the register
now, with nothing left reading this host's real home.

**Building and publishing a role's own application services, and building
the card itself, are still not this command's job — selecting the image now
is.** `ADR-054` is the settled record of this arc: provisioning selects a
system image and a role's applications from versions already published, and
never builds, signs or publishes anything, on any route. Step 2 above picks
a whole signed `card` image from the repository the way §4.8 already picks
`rivet-fleetd`/`rivet-proxyd` by digest, and `ADR-052` §7.2.3's "selecting,
unbuilt" intent is now built. A fleet server's card still gets `rivet-fleetd`
and `rivet-proxyd` the way §4.8 describes — resolved by digest out of a
fleet repository someone has already published into, at card-build time —
and building the card itself is still `tools/container/build-card.sh`
followed by `cargo rivet image` to publish it, run directly (§5, README
"Building or writing the card by hand"). See the History below for what
changed and why.

### 4.10 Ask which removable disk to write to — **BUILT**

**Built 2026-09-13 (3b): this step's verb is `cargo rivet write-image`,
renamed from `provision`** (§3.0, plan §5.2 item B, `.rulings/` R-162,
Wave 3b) once `provision` became the orchestrator that runs §4.5's interview
and then calls it — see the note after §4.9 for what that changes and does
not.

`cargo rivet write-image` enumerates rather than trusting a typed path.
`cmd/cargo-rivet/src/enumerate.rs` shells out to `Get-CimInstance
Win32_DiskDrive` on Windows and reads `/sys/block` on Linux; anywhere else it
refuses by name and says `--device` cannot be checked. Every disk it reports
carries its size, its manufacturer's model string, and — on Windows, via
`Win32_DiskDriveToDiskPartition` and `Win32_LogicalDiskToPartition` — the
drive letters currently mounted from it, which is the detail a person actually
recognises a card by.

**With `--device` omitted and `confirm` given — or on `cargo rivet
provision`'s interactive run with neither (§4 step 6) — the removable disks it
found are printed as a numbered menu and the operator must type a number — there is
no bare-Return default**, unlike every other menu in this tool. `SPEC-05`'s own
words ruled that out: *"MUST NOT ... default to one."* With `--device` given,
the path is looked up in the same enumeration rather than trusted outright.

**A disk the enumeration does not report as removable is refused, and there is
no flag that overrides it** — the refusal names the repair: unplug and replug
the card, or write it with an external imager deliberately. This closes the
gap the module's own header comment used to claim was already closed and was
not: `AGENTS.md` §10.4's *"the cost of getting it wrong is the development
host's own disk"* is exactly the failure a bypassable check would reopen.

**Naming the device is not the whole guard, and `confirm` alone is not
either** — `AGENTS.md` §10.4. Before anything is written, the plan is printed
and the operator is asked `Write it? [y/N]` — default no, a bare Enter writes
nothing. `confirm` as the last word on the command line skips that one prompt
and nothing else: the plan and the line explaining that administrator is about
to be requested (§4.11) still print. **The guard that catches a slip on the
menu or a mistyped `--device` is not a retyped identifier** — it is the
elevated child in §4.11 re-enumerating on its own account and refusing
anything not removable, whatever this process believed. The unelevated half
could be wrong about which disk is which; no amount of typing or clicking in
that half guards against that, which is why the check moved rather than being
removed.

### 4.11 Write the image — **BUILT**

The image is opened without `create`, so a path that does not already exist —
a typo — is refused rather than filled with a multi-gigabyte file of the wrong
name. Before the write starts, the disk is measured against the enumeration's
own size and refused if it does not fit, rather than discovering that two
gigabytes into an already half-overwritten card.

**Raw whole-disk access needs administrator, and the write happens in a
second, short-lived, elevated process because of it.** Measured 2026-09-09: an
unelevated shell was refused even a *read* of `\\.\PHYSICALDRIVEn`, while
volume handles like `\\.\E:` opened for read and write without complaint.
`cargo rivet write-image` itself stays unelevated — elevation is a property of a
process token at creation, and a process cannot raise its own — and once
`Write it? [y/N]` is answered yes, it prints the line explaining that Windows
is about to ask for administrator and relaunches itself, elevated, under
`provision --elevated-write`, from its own `current_exe()` path. One binary is
trusted; no second artefact is kept in step.

**The ruled sequence, in order, and the decision point is step 3:**

```
PLAN — nothing has been written.

  image    out/rivet-0.1.80-v0.1.77-99-g7d8eb56-aarch64-rpi4-card.img
           2.00 GiB

  device   \\.\PHYSICALDRIVE2
           Generic STORAGE DEVICE USB Device — USB, removable
           14.91 GB, currently mounted as E: F: G: H:

  WRITING THIS ERASES EVERYTHING ON \\.\PHYSICALDRIVE2.
  There is no undo and no trash.

Write it? [y/N] y

Windows will now ask for administrator, because writing a whole disk needs it.
Confirming that prompt starts the write; cancelling it writes nothing.
```

The UAC prompt is not the decision point — it arrives already explained, which
is what stops it being a dialog somebody clicks through on reflex.

**Handles do not cross the elevation boundary.** `ShellExecuteEx`'s `runas`
verb — what `Start-Process -Verb RunAs` shells out to — cannot hand a child
the parent's stdout, so the child gets a fresh console or none. Progress and
the result are files, not a pipe: a progress file the parent polls and
renders, and a small result file the child writes before it exits, both under
the system temporary directory with unpredictable names and removed by the
parent afterwards.

**The elevated child does not trust its arguments.** It re-enumerates in its
own context with the same mechanism as §4.10 and refuses any device that is
not removable, whatever the unelevated half believed — the check named in
§4.10 as the one that replaced the retyped identifier.

**The elevated child locks and dismounts the disk's volumes, or takes the
disk offline, before writing, and restores it afterwards.** A raw write to a
disk with mounted volumes is refused or, worse, reported successful while a
filesystem driver writes behind it — the step an operator cannot reasonably
be expected to get right by hand. `Set-Disk -Number N -IsOffline $true`
handles both named alternatives at once: taking a disk offline dismounts
every volume on it in the same call.

**Cancelling the administrator prompt is an ordinary outcome.**
`Start-Process -Verb RunAs` raises when the prompt is declined, and that is
caught and reported as *nothing was written*, exit zero, no stack trace.
Somebody who read the plan and changed their mind behaved correctly.

**Linux is a different problem, not the same one.** There is no mid-run
hand-off for an already-running process to request — Windows's works only
because `ShellExecuteEx` starts a NEW process with a new token. So this checks
whether it is already root, and if it is not, says so plainly and asks the
operator to re-run the exact same command with `sudo` in front of it, rather
than pretending the two platforms share a mechanism. Already root, the write
happens in this one process with nothing to hand off to.

### 4.12 Verify the image — **BUILT**

**The image is hashed with BLAKE3 as it is written; the device is read back
and hashed the same way; the two digests are compared.** Both directions move
in fixed-size chunks, so a multi-gigabyte image is never held in memory at
once. The device is `sync_all`-ed before the read-back opens a fresh handle,
so the comparison measures the media rather than the write's own page cache
(`AGENTS.md` §5's rule about never caching the kernel's verdicts, applied to a
disk rather than a network interface).

A short write, a short read-back, or a digest that does not match are each
reported as an error stating plainly that **the card is NOT usable** — the one
fact SPEC-05 named as what an operator staring at an unbootable card actually
needs restated, rather than a byte count they have to interpret themselves.

**The verify half stands alone.** `cargo rivet write-image --device <id>
--image <path> --verify-only` reads the device back and hashes it against the
image with nothing written first, for checking a card an earlier run already
wrote. It takes the same elevated path as a write on Windows — reading a whole
`\\.\PHYSICALDRIVEn` needs administrator exactly as much as writing one does —
minus the offline/online dance, since nothing is written.

**A second, weaker check needs no administrator at all.** Volume handles open
unelevated, so once the elevated child restores the disk online, the
unelevated parent reads `<letter>:\rivet.toml` off whatever drive letters
Windows assigns the two boot partitions — the same way any USB drive is read
every day — and confirms the document landed on both, byte for byte. This is
the check that answers *"will this card boot configured,"* which is the
failure that cost a whole bring-up on 2026-09-07: a card provisioned onto only
one boot partition forgets its configuration at its first update. It is
corroboration appended to the report, never a reason to fail the run — the
whole-disk digest above already covers these same bytes.

---

### 4.13 Backing up what §4.0 does not — **BUILT**, 2026-09-14

**Its own number, appended rather than renumbered into the twelve** — rule 6
(`AGENTS.md` §2) again, the same reason §4.0 sits outside them. Closes the
backup gaps `skills/taking-and-restoring-a-backup/SKILL.md` named
(`.rulings/` R-181, R-183) beyond §4.0's own authority backup: the two
signing seeds, the fleet repository's object store, a fleet server's own
secret store, and restoring an application's own data.

```text
cargo rivet key backup <backup-dir>
cargo rivet key restore <backup-dir> [--replace]
```

**Passphrase-encrypted, unlike §4.0's plaintext authority backup** —
`.rulings/` R-183 D3. A stolen CA backup is recoverable (this document's own
twelve-step re-mint); a stolen seed is not, until every board's trust store
ships an image update naming a new key (`ADR-028` §5.5.4). Asks for the
passphrase twice, no echo, from a real terminal only — no flag, no
environment variable. Refuses a wrong passphrase or a tampered file, writing
nothing either way; a forgotten passphrase makes the backup permanently
unusable, and the tool says so in its own output. `ADR-028` §5.8.3 and
`ADR-009` §3.10 have the encryption format and the crate survey.

```text
cargo rivet fleet objects backup <backup-dir> [--all] [--replace] [--at DIR]
cargo rivet fleet objects restore <backup-dir> [--replace] [--at DIR]
```

**Referenced digests by default** — what `manifest.toml` currently names,
not every digest `objects/` has ever held — `--all` for the whole historical
tree. Restore recomputes every object's own BLAKE3 before writing anything
and refuses a mismatch by name. `ADR-052` §7.3 has the design; §6.1's sizing
rule for a machine's own store is the identical argument applied to a
backup's default scope.

```text
cargo rivet backup-secrets  --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]
cargo rivet restore-secrets --host HOST:PORT --identity DIR --pin HEX --from DIR [--replace] [confirm]
```

**A booted board's `/state/secrets`, pulled over the control plane, sealed
under a typed passphrase, and put back onto the same board only** —
`.rulings/` R-183 D5 and R-185, `ADR-030` §4 and §5. Run it once the fleet
server answers, because the device identity and `peers` this document's steps
put on the card are in that store. The backup is one file,
`secrets-backup.enc`, in the envelope `cargo rivet key backup` uses: the
passphrase is typed twice at backup and once at restore, there is no flag for
it, and **a forgotten passphrase loses the backup**. Inside, the manifest
records the board's device public key; a restore with another board's `--pin`
is refused — a board re-carded with a new identity is another board — as is a
wrong passphrase, an altered backup and a `peers` file that would lock out the
restoring identity. Nothing is restarted: a restored file is live at the
board's next boot.

```text
cargo rivet restore --host HOST:PORT --identity DIR --pin HEX --service NAME --from FILE [confirm]
```

**One application's own data, sent back to the `restore-path` its signed
manifest declares** — `.rulings/` R-183 D1, `ADR-051` §12. The file is what
`cargo rivet backup` stored; it is checked against the application's declared
`backup-format` before anything is sent, and the application replaces its own
data — nothing on the machine is stopped. An application that declares no
`restore-path` is refused by name. As of 2026-09-14 TempMon and
Reliquary declare none (R-183 D2 is that work), so against this fleet the
command refuses each of them. It does not block a provisioning run.

---

## 5. What a fleet server's document MUST carry

**These are defaults of the ROLE, not things an operator is expected to know.**
Each one below was absent on 2026-09-07 and each cost a diagnosis cycle.

* **`[control-plane] listen`** reachable from the network — `0.0.0.0:7777` for
  a machine administered remotely. **The default of `127.0.0.1:7777` produced a
  headless board nobody could reach**, and the only recovery was rebuilding the
  card. **A default that bricks a headless machine is a defect, not an
  operator's omission.**

  **BUILT.** A `server` or `member` document binds every interface; a roleless
  one keeps loopback, and both write the table out rather than leaving it
  absent. `ADR-004` §6.2.1 is the decision and carries the two things the
  requirement did not anticipate: that the **schema's** default must stay
  loopback, because it is the fallback for a machine whose document cannot be
  read; and that a non-loopback listener is only correct on a card that carries
  a TLS identity, since `rivet-mcpd` refuses one without it and the machine gets
  no control plane at all. The tool says both on screen.

  **BUILT for `rivet-fleet` too, 2026-09-09.** `rivetos_register::Device::fleet_role`
  carries the same answer, and `cmd/rivet-fleet/src/compose.rs`'s
  `services`/`render` apply the same two defaults from it — but only when the
  device's own `listen` is absent, since the register's merge rule is *the
  device wins*. A record composed before this field existed, or one whose role
  nobody has answered, gets no role default at all rather than a guess: absence
  writes no `[control-plane]` table, exactly as before, so every register on
  disk today still composes the bytes it composed yesterday. `set fleet-role`
  is the verb that answers it for an existing device.

  **`ADR-059` §8, 2026-09-22: the role is `server` or `member` — two words,
  not three.** `"stand-alone"` named no behaviour of its own even while it was
  offered (every default above already treated it exactly like no role at
  all), so both `rivetos_register::FleetRole` and `rivet_config::FleetRole`
  dropped the variant and the word is no longer offered by either the
  provisioning interview's Fleet question or `rivet-fleet set fleet-role`. A
  document or a register record still on disk with the retired spelling keeps
  loading, as no role, rather than being refused — the two ratchets each keep
  their own backward-compatible read path for it.
* **`rivet-fleetd`, `rivet-proxyd` and `rivet-mdnsd` enabled.** **BUILT**, from
  the `Fleet` answer; a `member` gets `rivet-mdnsd`, for the reason it is on
  this list rather than as a courtesy — it is reached by name.

  **BUILT for `rivet-fleet` too, 2026-09-09**, from `Device::fleet_role`, as a
  third layer beneath the fleet's and the device's own: `services` inserts the
  role's set first, so an operator's `false` — on the device or on the fleet —
  still wins. A stand-alone or roleless device gets nothing from its role,
  matching the register's existing byte-stability claim.
* **`[applications.rivet-proxyd] upstream-name`, wherever `rivet-proxyd` is
  enabled. BUILT.** The table is written whenever the role enables the proxy,
  and `compose` **refuses** rather than composing a document without it — a
  document that enables the service and omits the setting is the one thing worse
  than not enabling it, so it cannot be produced at all now. The value is
  `<hostname>.<suffix>` for the first suffix in the authority's own certificate,
  which is the same subject `ca server` composes.

  With no setting, `cmd/rivet-proxyd` verifies its backends as the first DNS
  name on its own certificate (`ADR-060`), and refuses to start when the
  certificate names no host. It never composes one from the kernel hostname,
  because appending `.local` would assume a suffix `ADR-034` makes a fleet
  property. The composed setting stays: it makes the document say which name
  the front door verifies, rather than leaving it to whichever certificate the
  machine happens to hold — and a front door that could not find its name is
  the state §10.5 records being fixed by hand after the card had booted.

  **This bullet is late, and its absence is the same defect this document is
  about.** §10 is the manifest of everything done by hand on 2026-09-07 and
  `upstream-name` is *in* it, at §10.5, as one of five document edits made after
  boot. §5 listed the services and not the setting one of them cannot start
  without. **A specification that contradicts its own evidence section is worth
  less than no specification**, because it is trusted.

  The value is the name on the fleet certificate — `<hostname>.<suffix>` — which
  `ca server` now composes from the authority's own suffixes, so the tool
  already knows it at the moment it composes the document.

  **BUILT for `rivet-fleet` too, 2026-09-09, and by a DIFFERENT mechanism —
  said plainly rather than implied to match.** `rivet-provision` DERIVES the
  value, at the moment of composing, from the authority's own certificate.
  `rivet-fleet` has no certificate to read — the register holds no secret and
  `ADR-034` §3 keeps it that way — so `Device::applications` holds whatever a
  device's own record says, and `add document`'s `adopt` is what fills it:
  it copies `[applications.rivet-proxyd] upstream-name` out of a machine that
  already answers it, the same way it already copied a resolver's nameservers.
  `compose::provisioning_complete` refuses a FINAL document — an export, a
  card, a live write — without it, exactly as `rivet-provision` does; the
  permissive `compose::provisioning` stays gapless on purpose, because it is
  what `compare` (adoption's own engine) composes against, and a compose that
  refused there would make the one tool that fills the gap unable to run.
  **The two composers still disagree about where the value comes from, and
  this bullet does not paper over that** — see `## History`'s 2026-09-09 entry
  for what unifying them would take.
* **The credentials `rivet-fleetd` reads** — `server.crt`, `server.key` and
  `ca.crt` in its data directory (`cmd/rivet-fleetd/src/main.rs:55-59`). The
  server certificate is minted at this moment because this is when the hostname
  first exists, which the existing code already gets right.

  **BUILT**, onto the card, by §4.9's mechanism. `rivet-proxyd` is handed the
  same leaf under the three names *it* reads, because two certificates for one
  machine is a browser trusting one page of it and not another.

  **Built 2026-09-13 for a fleet MEMBER too** (`.rulings/` R-160 R25). A member
  enables `rivet-proxyd` (`ADR-050` §5.1) with no `rivet-fleetd` of its own, so
  `cmd/rivet-provision/src/stage.rs::compose` stages only the three files
  `rivet-proxyd` reads — its own leaf certificate and key, minted from the
  fleet CA for its own name, plus the same public root every machine's front
  door checks operator certificates against. `server.crt`, `server.key`,
  `ca.crt`, the operator hex pair and `register/` stay server-only.
* **`rivet-fleetd`'s own OPERATOR credential** — `operator-device.seed.hex` and
  `operator-device.crt.hex`, in the same directory
  (`cmd/rivet-fleetd/src/node.rs:34-36`), as **hex text** rather than raw bytes.
  **And its public key MUST be in `/state/secrets/peers`** on every machine it
  is to manage, including its own.

  **The server's own record in the staged register carries
  `role = "fleet-server"`** (`rivet_core::fleet_as_code::SERVER_ROLE`), and
  `rivet-fleetd` reads it to find its own control plane: fleet-as-code's git
  runs in that machine's `rivet-mcpd`, which `rivet-fleetd` asks with this same
  operator credential (`ADR-044` §5.5). Provisioning does not turn fleet-as-code
  on — `[applications.rivet-fleetd] fleet-repository` stays unset, and the one
  repository it declares, `[repos.fleet]`, is `ADR-052`'s, which fleet-as-code
  refuses to write.

  **This is a THIRD card-time artifact and it is the one nobody had noticed.**
  Without it the fleet portal lists machines and can drive none of them: every
  page answers *"this machine did not answer"*, naming the missing seed. A fleet
  server asks itself over the control plane exactly as it asks anything else —
  `/run/rivet/query` is `0600 root:root` and serves only `status`, so there is
  deliberately no self-shortcut (`node.rs:20-29`).

  **It cannot be repaired after the fact without a lockout risk**, which is what
  makes it card-time rather than a fix-up. `rivet-mcpd` builds its pin list in
  `tls_config` **at startup, before the socket is bound**
  (`cmd/rivet-mcpd/src/main.rs:470`), so a key added to `peers` takes effect
  only on restart — and a `peers` file that will not parse leaves the control
  plane refusing to bind. On a headless board that is the end of the
  conversation. Found 2026-09-07 on `rivet-01`, and deliberately left unfixed
  because the maintainer was away from a machine whose card he cannot pull.

  **BUILT.** `rivet-provision` mints the operator credential when it composes
  the document, stages both halves as hex text under the names `node.rs` reads,
  and **adds** the public key to the `peers` file in the identity directory the
  card will be built from. It only ever adds: a peers file is the whole list of
  who may reach a machine's control plane, and dropping a line from a headless
  board's card is a lockout whose only repair is another card. `rivet-01` is
  untouched by this and is still in the state recorded above.
* **`[repos.fleet]`, the git repository it serves the fleet repository's text
  as.** **BUILT 2026-09-14** (`.rulings/` R-184 D3): the interview writes
  `[repos.fleet] id = "<minted>"` into every `server` document and into no
  `member` or roleless one, the identifier minted at `w` in
  `rivet_core::identifier`'s grammar. `rivet-sysd` creates
  `/state/git/fleet.git` from it on first boot, `rivet-mcpd` serves it over the
  control plane (`ADR-035` §4), and `cargo rivet fleet push <host>` puts the
  workstation's fleet repository into it (`ADR-052` §9 step 4). The name is
  fixed and never asked. A document provisioning did not compose — a
  hand-written `--document PATH`, or the recorded document `--existing`
  rewrites — is written as it is, without one added; `ADR-054` §5.1 is the
  reasoning, and the summary names the gap for such a server.
* **`[network.resolver] nameservers` when its address is static. BUILT**, from
  the interview's Nameservers question (§4.5). A fleet server is the machine
  most likely to be given a static address, and without the table it resolves
  nothing. A DHCP document declares none, so its lease supplies the resolver.
* **Its certificate MUST carry every name the authority permits**, not a single
  literal passed on a command line. **BUILT:** `ca server <hostname> <dir>`
  composes `<hostname>.<suffix>` for every suffix, read out of `root.crt`
  itself; `--names` is the opt-in literal list. The menu already did this and
  the command line did not, which is `ADR-006` §3's co-equality broken in the
  most expensive place available.

### 5.0 THE TIMEZONE, and it must be ASKED not defaulted — **BUILT**

**Ruled by the maintainer 2026-09-07:** the timezone is set for his board now,
and the next time a board is provisioned it must be a setting offered to him.
*(`.rulings/` R-261.)*

**The provisioning menu MUST ask for the timezone, and the composed document
MUST carry the `timezone` key.**

**Why it cannot be left until later.** `/etc/localtime` is projected from the
`timezone` key **before any service starts**, into a tmpfs that is then
remounted read-only (`ADR-007` §3.5.1, §3.6.1). So a machine provisioned
without it keeps UTC, and setting the key afterwards changes nothing until the
machine is **restarted** — which is what `rivet-01` needed on 2026-09-07, with
Aura already on the television showing the wrong time.

**Why silence is the expensive failure.** `explain /etc/localtime` states it:
musl, Go and chrono all read that exact path when `TZ` is unset, and *"without
it every one of them keeps UTC **SILENTLY** — a machine whose document names a
zone and whose clock is an hour out with nothing in any log"* (`ADR-021` §9).
Nothing fails. Every timestamp is simply wrong, including the ones an operator
would use to diagnose it.

**It MUST NOT be guessed.** A zip code is already collected for other reasons
and a workstation has a zone of its own; neither is the machine's. Offer a
sensible default, ask, and record what was chosen.

**And Aura's instruction for changing it is wrong**, which is how this surfaced:
its General page says *"Change it with `rivet apply edit timezone <zone>
confirm`"*. There is no such verb — `apply edit` takes a path and
`--expect`/`--replace` or `--append`. `timezone` is a top-level document key, so
it cannot even be appended to the end of a document that ends in a table.
`examples/aura/aura-controller` is in this repository and this is a defect in
it. **It is still open** — nothing in this pass touched Aura.

**How the asking is done, since *"the menu MUST ask"* has a weak reading that
would satisfy the letter and not this.** The field begins **refused**, and `w`
will not compose while any field is: an unanswered zone is `fix 7` on the screen
with its reason beside it, and there is no way past it but answering. A field
carrying a plausible default would be answered by silence, and silence is the
defect being fixed. `UTC` is offered first and typing it is a recorded answer.

**And the name is held against zones that EXIST**, which is the requirement's
*"or at minimum against a known list"* and is the part that had no home in this
workspace. `rivet_core::timezone` enforces the grammar and says outright that it
cannot know whether a zone exists; `America/Chigago` therefore parses, projects,
maps nothing and produces a UTC clock. `cmd/rivet-provision/src/zones.rs` checks
a real `zoneinfo` directory when one is readable — the case in the build
container — and a generated list of the card database's own 599 names when one
is not, which is the case on Windows. It reports which of the two answered, and
a refusal names the zone the operator meant.

**BUILT for `rivet-fleet` too, 2026-09-09, and honestly weaker: no EXISTENCE
check.** `Device::timezone` holds whatever a machine's own document already
carries — `add document`'s `adopt` copies it in unvalidated, because the value
it copies has already passed through `rivet_core::timezone` once, on the
machine that answered it. `compose::provisioning_complete` refuses a FINAL
document without one, in the same words `rivet-provision`'s refusal uses.

**And now there is a way to answer it before a machine exists to adopt it
from.** `set timezone --device <name> --timezone <zone>` records the value
directly, checked against `rivet_core::timezone::Timezone` — the same grammar
`rivet-config`'s own parser enforces, so a value this accepts is a value the
board would accept too. **It stops at grammar**, and the refusal says so:
whether the zone actually exists needs a zoneinfo database, and
`cmd/rivet-provision`'s copy of one lives in a binary crate with no library
target, so nothing outside it can call the function that checks it — this
tool would have to duplicate the six-hundred-name table to check further, and
`ADR-011` gives that table one authority. A well-formed name for a zone no
image carries passes here and is reported by the machine's own projection
instead, at first boot.

**`ADR-034` §6.1's both-or-neither does not bind this verb, and the
2026-09-07 note above that speculated it would was wrong about why the verb
was missing.** That rule binds a value that is *how the tool reaches the
machine* — a wrong `[control-plane] listen` is self-sealing, because the
register that holds the mistake is the same register the correction would
have to be sent through. A timezone carries none of that: recording one
wrong in the register changes nothing about whether `set timezone` can be
run again, so it composes like `fleet-role` and `pin` do — register-only,
taking effect the next time a document is exported, built, or written.

### 5.1 A kiosk machine additionally — **BUILT, with a gap that is named**

`[kiosk]`, `rivet-launcher` enabled, and `auto-run` naming an application that
**is installed**. Naming an absent application takes the display and produces a
black screen with no message on it.

**What is built.** A `kiosk` machine gets a `[kiosk]` table; the menu asks which
application takes the display and offers `rivet-launcher` by number; and
**whatever is named is also enabled** in `[services.<name>]` — the supervisor
starts nothing the document has not enabled (`ADR-015` §3.2.1), so an `auto-run`
naming an installed but disabled application is the same black screen by a
different route. Blank is accepted as a real answer and writes an empty table,
which `rivet_config` reads as *no application takes the screen*.

**BUILT for `rivet-fleet` too, 2026-09-09.** `Device::kiosk_auto_run` holds the
name, the composer (`crate::compose::render`, in `cmd/rivet-fleet`) writes it
back out as `[kiosk] auto-run = "…"`, `add document`'s `adopt` fills it in from
a machine that already answers it, and `set kiosk-auto-run --device <name>
--application <name>` answers it directly for a record with no machine to
adopt from yet — the same reason `set timezone` above exists. **This does not
close the gap below; it gives the register somewhere to keep the answer once
the gap is accepted.** Unlike `set timezone`, there is nothing here to check
even at the grammar level: `rivet-config`'s own parser takes any string.

**The gap, stated rather than guessed past.** Nothing at card time can know
whether the named application will be installed — none of them are, and the
manifest that would say so is written by an install that happens afterwards. So
the tool asks and prints the risk,
and does not compose `rivet-launcher` unasked on the grounds that it is usually
right. Closing this needs the card build and the document to know about each
other, which is §4.8 and §4.9. `ADR-004` §6.2.4 carries the reasoning. **This
is unchanged by `rivet-fleet` gaining a field for the name**: recording a
value and validating it are different questions, and only the first is
answered here.

---

## 6. What this specification rules out

* **No hand-edited TOML**, by the maintainer or by an agent, at any point.
* **No `apply install-secret` after boot** to make the fleet server work.
* **No agent required** at any step.
* **No shell script standing in for a missing tool feature.** If a step's answer
  is *"run this script that does nine things"*, those nine things belong in the
  tool.

---

## 7. Acceptance

**The specification is met when the maintainer, alone, on a workstation reset by
§2, performs §4 and the card boots to `rivet-fleetd` `started` with its fleet
visible at `https://<host>:4443/`.**

**This has NOT been measured.** As of 2026-09-07 the card carries everything in
§10.2, §10.3 and §10.4 and nothing has been booted from one. `ADR-049` §9.2 is
the honest statement of what is proved and by what method, and names the three
things that close it: a card, a test board, and the command below.

**§4.8 and §4.9 now know about each other, and what that does and does not
close.** `--fleet-repository` (§4.8) resolves a fleet server's document into
`rivet-fleetd` and `rivet-proxyd`, carries them, and `ADR-049`'s staging bundle
(§4.9) already delivers their credentials into the data directories the
resulting install creates — so a fresh card built with both is, on paper, one
whose first boot carries the payload, installs it at the first convergence
(`ADR-050` §4.1, §10) and receives its credentials at the discovery pass that
install triggers. **This has still NOT been measured**, and cannot be measured
under QEMU: first boot's read of the carried payload rides the same
boot-partition mount `rivet.toml` itself is read through
(`cmd/rivet-initd/src/provision.rs`), which needs
`/proc/device-tree/chosen/bootloader/partition` — a property `ADR-026` §4 says
`-M virt` does not emulate at all. `ADR-049` §9.2 and `ADR-050` §11.2 are the
honest statement of what is proved and by what method, and name the three
things that close it: a card, a test board, and the command below.

Measured by:

    rivet run --host <addr> --identity <dir> --pin <hex> -- get status

`rivet-fleetd` `"state":"started"`, and `/api/fleet` returning the records that
were injected. **The portal's HTML is not evidence** — it ships its
empty-register sentence as a JavaScript string at all times, and reading the
page instead of the endpoint produced two wrong conclusions in one afternoon.

**Wave C3 (2026-09-14) widens what a card carries and still does not move
this line.** `ADR-054` §3–§5's injection and verification are unit-tested and
proven against a real scratch fleet repository with a real signed artifact
— never against a board. `ADR-054` §8 is this wave's own honest account,
naming the identical three things §9.2 already names above.

**The fleet server's repository is a separately proven claim, and it is not
this acceptance either** (2026-09-14, `.rulings/` R-182/R-184). A server's
card carries the repository's text and none of its objects. That the server
creates its `[repos.fleet]` repository, serves it, and takes `cargo rivet fleet
push` is proven on a QEMU guest booted with that declaration
(`tools/qemu/adr-035-git-proof.sh`, `ADR-035` §4.1) — not from a written card,
and not on a board. That `cargo rivet fleet sync` then fills the server's
repository store with the application objects its manifest names, refuses a
tampered or wrong-kind object, and prunes what the manifest stops naming is
proven the same way (`tools/qemu/adr-052-objects-proof.sh`, `ADR-052` §7.4.7).

---

## 8. Out of scope

* **Adopting a machine already running.** `ADR-034` §6.2's `add document`
  covers this and cannot reach a machine whose record has no address — a
  deadlock recorded in the findings, and a related but separate piece of
  work. **Narrower than before 2026-09-13**: enrolling a fleet **member**
  during ITS OWN provisioning is now built (`.rulings/` R-178 Q7,
  §4.5–4.12 step 4/6) — `cargo rivet provision` pre-enrolls a server or
  member into the operator's register before the card is even written.
  What stays out of scope is adopting a member that already exists and
  already booted, which needs the network round trip `add document`
  performs and this specification never has.
* A **standalone** machine with the workstation acting as fleet server.
* Anything after first boot.

**These are named so their absence is deliberate.** Thursday is the fleet
server; the rest follows it.

---

## 9. Known defects that block this, verified 2026-09-07

**All three are fixed and are in the History.** Nothing in the list that blocked
this document on 2026-09-07 is still open.

## 10. The manifest — read off `rivet-01` on 2026-09-07, not remembered

**A working fleet server was reached by hand. This is the list of what the hand
did, and therefore the list of what the card must carry.** Every line was read
back off the running machine with `get file`, not reconstructed from a
transcript.

### 10.1 Per-application TLS material — 5 applications × 3 files

`tls-cert` (524 B), `tls-key` (246 B), `tls-root` (672 B), in each
application's data directory, owned by it:

`rivet-proxyd`, `aura-controller`, `tempmon-rivet`, one more adopter,
`reliquary-rivet`.

**`cargo rivet secrets APPLICATION … confirm` already does this**, one command
per application, and it is the one part of the hand-work that was not a defect —
only the fact that it happens after boot rather than being on the card.

**Everything in §10.2, §10.3 and §10.4 is now composed at card time** by
`ADR-049`'s mechanism, and §10.5's `upstream-name` by §5. §10.1's other four
applications and §10.5's `deploy --enable` tables are still after-boot work, and
§10.6 says why the first of those is not a defect.

### 10.2 `rivet-fleetd`'s own set — 5 files under 3 different naming schemes

| file | size | what it is |
| :--- | ---: | :--- |
| `server.crt` | 524 B | its TLS certificate, **not** `tls-cert` |
| `server.key` | 246 B | its key, **not** `tls-key` |
| `ca.crt` | 672 B | the authority, **not** `tls-root` |
| `operator-device.seed.hex` | 64 B | its operator private key, **hex text** |
| `operator-device.crt.hex` | 562 B | its operator certificate, DER as **hex text** |

**Three naming schemes in one directory**, and no tool installs the last two.

### 10.3 The register

`register/register.toml` beside `register/devices/<id>.toml`, in
`rivetos_register` layout, owned by `rivet-fleetd`. Two directories, two files,
five ownership changes.

### 10.4 One line in `/state/secrets/peers`

`rivet-fleetd`'s public key, so the machine will accept its own fleet server as
a caller. **Takes effect only on restart** (§5).

### 10.5 The machine document, after the card had booted

`[applications.rivet-proxyd] upstream-name`, `[kiosk] auto-run`,
`[services.rivet-bluetoothd]`, and six `[services.*]` tables written by
`deploy --enable`.

### 10.6 What was NOT hand-placed, and it matters

`config.json`, the reading source's credential and `weather-cache.json` in `aura-controller`;
`tempmon.db` and its WAL; `admin.secret` and `blobs/` in `reliquary-rivet`.

**Every application created its own state correctly, unaided.** The platform's
data-directory contract works. **What is missing is only the per-machine
material the platform is supposed to hand an application before it starts** —
which is exactly the scope of Wave 3 and no wider.

### 10.7 An error left in place, recorded rather than tidied away

`rivet-fleetd`'s data directory also holds `tls-cert`, `tls-key` and `tls-root`
— **installed by an agent that guessed the naming scheme before reading
`main.rs`**, and never read by anything. They are dead files on a production
machine.

They are left there deliberately until the maintainer decides, because removing
evidence of a mistake from the machine it was made on is how §9's defects stop
being findable. **`rivet-reset` and a fresh card are what clear them.**

---

## 11. Other known defects

### 11.1 `apply create-file` cannot write multi-line content

`<text>` is *"OPTIONAL and one word"*. The working combination —
`apply edit <path> --append <text>`, which creates an absent file — is
undocumented and neither verb points at the other. Relevant only if §4.9 is
implemented over the control plane rather than onto the card, which it should
not be.

---

## History

### 2026-09-23 — §5: the proxy had no default for `upstream-name`

§5 said:

> ~~`cmd/rivet-proxyd/src/main.rs:274` has no default and no derivation … so
> the process exits, retries five times and is marked `failed`. **A card
> composed today therefore boots to a fleet server whose proxy is dead**~~

`ADR-060` made the proxy part of the image and gave it a default: the first DNS
name on its own certificate. Provisioning still composes the setting, for the
reason §5 now states.

### 2026-09-14 — R-189 D2: one identity per machine

The Identity field defaulted to `~/.rivet/identity/board` for every machine,
so `rivet-08ecc` was provisioned with the same device key as the register's
older `board` record. Step 5 compared identifiers and hostnames only, and that
record's identifier was minted at random by `rivet-fleet add device` rather
than derived from its pin, so the shared key passed. Every `w` also minted a
fresh `rivet-fleetd` operator credential, so the identity's `peers` gained a
key on each repeated or refused run. §4.5 now defaults and mints per hostname,
the interview and step 5 both compare pins, and `w` reuses the staged operator
credential.

**Verified by:** `cargo test -p rivetos-provision` (`identity_tests.rs`, the
peers test in `stage_tests.rs`, and `tests/identity_at_w.rs`, which pipes
answers into the real binary under a scratch `RIVET_AT`), `cargo test -p
cargo-rivet` (the pin refusal on the `--identity` route, and `--existing`
finding a machine's own identity), and `pty_interview.rs` under a real ConPTY,
which minted `identity/<hostname>` at `w` and injected it. **Unproven**: a card
written with a minted identity booting and answering its operator; nothing
here touched the real register or `identity/board`.

### 2026-09-14 — the bare interactive `cargo rivet provision` goes on to the device

§4 step 7 read that recording happens *"on a route that targeted a device
(`--device PATH` or `confirm`; an image merely prepared with neither is not a
finished card)"*, and with neither flag step 6 stopped at *"the image is ready
at …"*. The maintainer's first real run typed the bare command, finished the
interview, and was left with that line naming a working copy the run then
removed — against R-174's requirement that the run end with him pulling a fully
provisioned SD card. Step 6 now offers the device menu and asks `Write it? [y/N]` on any run
with a terminal, and step 7 records on a verified write rather than on which
flags were given, so a declined question or a cancelled administrator prompt
records nothing either.

**Verified by:** `cargo test -p cargo-rivet` (the device-step, question and
rerun-command tests in `write_image_tests.rs` and `provision_tests.rs`) and
`cmd/cargo-rivet/tests/pty_interview.rs` under a real ConPTY, which reached the
no-removable-device ending on this host. **Unproven**: the device menu and a
typed `N` under the pty (no card was attached), and the real administrator
prompt, write and record — the maintainer's next real run.

### 2026-09-14 — R-182 and R-184: a server's card carries text only, and its document declares `[repos.fleet]`

§4.5–4.12 step 6 read that a fleet server's document *"additionally carries
the fleet repository's text half and its `application`/`library` objects"*.
The staging bundle is hex text capped at 5 MiB and a real server's two
applications alone overran it, so a real server provision was refused;
`.rulings/` R-182 ruled the card carries text only and the objects follow over
the control plane after first boot. R-184 found git serving already built and
ruled `[repos.fleet]` a role default of a server's document, which §5 now
lists, and step 8's summary names `cargo rivet fleet push` and then `cargo
rivet fleet sync` for the objects (R-184 D1). §4.9 and §7 gained a sentence
each for the same change. `ADR-054` History carries the superseded §5 text.

**Verified by:** `cargo test -p cargo-rivet --bin cargo-rivet` (carry, provision,
fleet push and fleet sync suites) and `cargo test -p rivetos-provision`; the
server's repository, the push and a second client's clone by
`tools/qemu/adr-035-git-proof.sh`, and the objects' sync, refusals and prune by
`tools/qemu/adr-052-objects-proof.sh`, each on a QEMU guest. **Unproven**: a
board.

### 2026-09-14 — `--at DIR` reaches the interview and the register, not only `cargo-rivet`'s own steps

Found building `cmd/cargo-rivet/tests/pty_interview.rs` (R-178 P6's own
proof, the entry below-below this one): `cargo rivet provision --at DIR` set
`RIVET_FLEET_REPOSITORY` on the `cmd/rivet-provision` child and nothing
else. That binary read `HOME`/`USERPROFILE` directly for its CA, its
saved-answers notebook, the document and `ADR-049` staging bundle it wrote,
and its default "Identity" suggestion — every one of them this HOST's real
ones, `--at` or no `--at`. The harness's first version, before it worked
around this by hand, surfaced `resume which?` naming a real saved notebook
from the maintainer's own machine before a single keystroke was sent.

**The fix.** `cmd/rivet-provision` gained `rivet_root` (that crate's own
module doc): `RIVET_AT` first, falling back to `$HOME/.rivet`/
`$USERPROFILE\.rivet` only when unset — unchanged for a direct run, with
nobody setting the variable. `cargo rivet provision`'s `SubprocessInterview`
now sets `RIVET_AT` on the child, always, to the SAME `at` it already
resolves for its own credentials survey and image verification.
`breadcrumb_path` (`cmd/cargo-rivet/src/provision.rs`) was a second,
independent divergence sitting right beside the first — it re-resolved
`credentials::root()` on its own rather than reading the `at` this run had
actually been given — and now takes `at` directly from the same place.

**The register needed a different rule, not the same one.**
`crate::device::default_register_dir` resolves `rivet-fleet`'s register
through `ignition::config` — `%APPDATA%\rivet-fleet\register` on Windows —
which has no relationship to `HOME`/`USERPROFILE` or to `--at`; left alone,
a scratch `--at` run's enrolment (R-178 Q7) would still land in the
OPERATOR'S REAL register. `register_root_for` branches on the LITERAL
`--at DIR` flag, never the resolved credential root (which is `Some` even
without `--at`, via `credentials::root`'s own default) — branching on the
resolved value instead would have silently moved every ordinary, no-`--at`
provisioning run's register out from under it. Given `--at DIR`, the
register is `DIR/register`; without it, nothing changes, `RIVET_FLEET_REGISTER`'s
own override included.

**The fleet repository's own default took the resolved credential root
instead**, and this one is safe to do unconditionally: `fleet_repository_root`
fell back to `rivetos_fleet_repo::default_root()` — a fresh `HOME`/
`USERPROFILE` read regardless of `--at` — and now falls back to
`at.join("fleet")`. `at` itself defaults to `credentials::root()`, the
identical `$HOME/.rivet` shape `default_root` computed independently, so the
ordinary answer is unchanged byte for byte; what changes is a `--at`-only
run (neither `HOME` nor `USERPROFILE` set, previously refused outright for
image selection) now resolving a repository, and a scratch `--at` run
defaulting to a scratch repository rather than the operator's real one.

**Verified by:** `cargo test -p rivetos-provision` (99 passed) —
`resolve_rivet_root`'s own four cases (`RIVET_AT` wins over `HOME`, falls
back to it when unset, an empty `RIVET_AT` is treated as unset, neither set
is `None`), pure. `cargo test -p cargo-rivet --bin cargo-rivet` (529
passed) — `breadcrumb_path` reading from under `at` rather than
`credentials::root()`; `fleet_repository_fallback` and `register_root_for`'s
own under-`--at`/unchanged-by-default cases, pure. `cargo test --release -p
cargo-rivet --test pty_interview -- --ignored --test-threads=1` (3 passed)
— the same three real-terminal proofs this document already records for
R-178 P6, re-run with `HOME`/`USERPROFILE` pointed at a directory that must
never come to exist rather than a scratch directory the harness kept in
hand-maintained lock-step with `--at`, and an assertion after each run that
nothing was ever created there. `cargo clippy -p rivetos-provision --all-targets`
and `cargo clippy -p cargo-rivet --all-targets` — no new warning.
**Unproven, and unchanged by any of this**: that the register actually
gains a record under `--at` in a REAL ConPTY run — R-176 Q10 gates
enrolment on an actual device write, which this harness (and this session's
own hard rules) never performs; the register-under-`--at` wiring is proven
directly instead (`register_root_for`'s own pure test, and
`RealFleetRecorder` against a real, scratch `rivetos_register` register in
`provision_tests.rs`'s existing `scratch_end_to_end_*` coverage) — and, as
always, whether a card produced this way boots, which §7 already records as
unmeasured.

### 2026-09-14 — Wave C3: the injection widened, carried payloads verified, a fleet server carries its repository, `ADR-054` recorded

Until this entry, step 6 ("Calls `write-image`") wrote only `rivet.toml`
onto a selected image's boot partitions — the staging bundle, this card's
own identity and every carried `app-<product>.rvt` existed only for a
**fresh** `cmd/rivet-image` build (§4.8, §4.9), never for the prebuilt image
provisioning actually selects and writes. A card written by `cargo rivet
provision` therefore booted with a document and nothing else it needed.

`ADR-054` is the new, dedicated record of the R-174→R-175→R-176→R-177→R-178→R-180
arc this document's own History (below) had been carrying piecemeal across
four separate entries; §4.5–4.12 above now cites it rather than restating
its reasoning. §1.1, §4.5–4.12 step 6/8 and §7 above were rewritten in this
pass to describe the widened injection, `ADR-050` §4.6's workstation-key
verification, and the item-5 summary as current fact — nothing here was
previously described differently, so nothing is struck; this is the entry
`ADR-054`'s own two-layer rule asks for when a document's true-now state
gains a fact rather than loses one.

### 2026-09-13 — R-178 items 1–8 (Wave C2): the image now comes from the fleet repository, selected before the interview

Until this entry, §4.5–4.12's step 5 (now step 6) took its image from
`--image PATH` or the same `out/*-card.img` menu `write-image` always
offered — the shape this command had before R-174, restored by R-175/R-176's
revert (the entry below-below this one). Nothing chose it FROM the fleet
repository R-177 had, by then, already given `cargo rivet image` a way to
publish into.

The design draft this wave started from
(`provision-design/DRAFT.md` §3, in the maintainer's scratchpad) proposed inferring
the platform from this workstation's own cross-build target
(`rivet_adapter_image::PiFirmware::platform_token`), asking only as an
override, and treating a schema-version mismatch as a printed note rather
than a refusal. The maintainer overruled both when the platform question was
put to him: if the platform is not indicated and several are available, he
expects to be prompted for the platform he is provisioning for before the image
is selected. *(`.rulings/` R-178.)*

`.rulings/` R-178 ruled: the platform is asked, never inferred, when the
repository holds images for more than one; a schema mismatch is listed and
marked, never a silent note, because a board refuses a document on any other
schema version outright (`rivet-config`'s own `parse.rs`); selection happens
before the interview, so a refusal costs no typing; and `--image` widens to
accept a fleet-repository digest or an unambiguous prefix, checked against
the platform and schema like a menu pick, so an agent with no terminal can
still name a repository image directly. The reasoning for each now lives in
§4.5–4.12's body above, as a positive statement of what is built rather than
struck text here.

### 2026-09-13 — R-180: the redirect for an already-provisioned hostname, and static addresses enrolled

The entry below this one shipped Q7/Q10's duplicate-enrolment refusal with
nothing else to do about it — a person redoing a card after a failed boot,
or rewriting `rivet-01`, had no way past the refusal at all. Asked whether
the same identity should instead update the record, the maintainer ruled
against it plainly — a machine already provisioned cannot be provisioned
again (`.rulings/` R-180) — and asked for a redirect instead, naming the shape
himself: `cargo rivet provision --<flag> "hostname"`, taking that hostname's
own desired-state document onto an image selected the ordinary way. He
confirmed the flag as `--existing <hostname>`.

**The redirect.** Step 4's duplicate refusal now names
`cargo rivet provision --existing "<hostname>" [--image PATH] [--device
PATH] [confirm]` in full — `redirect_for` builds the sentence, so the two
duplicate shapes (a repeated identifier, a repeated hostname) cannot drift
into naming it differently. Given `--existing HOSTNAME`, the command skips
the interview, the credentials preflight and the enrolment check entirely,
locates the document ALREADY recorded for that hostname
(`RealExistingDocument`, reading `<fleet repository>/documents/
<host>.provisioning.toml` — never `~/.rivet/provision/`'s own copy, which
can hold a document no verified write ever carried through), writes it back
to the same local path the interview's own `w` key would have used so the
existing staging-bundle check finds the ORIGINAL material beside it (or
refuses by name, on a different workstation than the one that provisioned
it), and then runs the ordinary write. It commits nothing and enrols
nothing, on every route through it, successful or not — the whole point is
that the machine is already on record.

**Static addresses enrolled.** Asked in the same pass whether enrolment
records a static address, the maintainer ruled yes, mirroring
`cmd/rivet-fleet`'s own `add document` shape (`method`/`address`/`gateway`
per interface, keyed by name). `enrolment_device` now populates
`Device::interfaces` for every statically-addressed interface in the
document; a DHCP one records nothing, as it always has, since the fleet
finds it by name instead.

**Verified by:** `cargo test -p cargo-rivet --bin cargo-rivet` (452 passed)
— the redirect route through `run_with` with fakes (no interview, no
preflight, no enrolment planning, no commit; the write happens; an unknown
hostname refused by name); `RealExistingDocument` against real scratch
files (found and materialised locally; an unknown hostname refused; no
fleet repository refused); `redirect_for`'s exact sentence, and that a real
`RealFleetRecorder` duplicate refusal contains it; `extract_existing`'s
parsing; and `enrolment_device`'s static-address and DHCP-records-nothing
cases, including a static address with no gateway. `cargo clippy -p
cargo-rivet --all-targets`: no new warning. **Unproven, and unchanged by any
of this**: the interactive interview and the real write path, same as the
entry below.

### 2026-09-13 — R-178 P3/P4/Q7/Q10: version selection, and recording after the write

The entry below this one left R-176's model half-built: provisioning would
select from the fleet repository, but nothing yet chose which VERSION of a
role's required application to carry, and the fleet-repository commit still
happened at the interview's own `w` key — before a card existed, contradicting
R-176 Q10's ruling that recording happens *after* the write. This closes both
gaps, without touching R-176's still-unbuilt piece (selecting a whole card
*image* from the repository, which stays `--image PATH` or `write-image`'s own
menu for now).

**P3 — the interview picks application versions.** `cmd/rivet-provision` gained
`versions.rs` (a third reader of `manifest.toml`, beside `cmd/cargo-rivet/src/
fleet.rs` and `cmd/rivet-image/src/fleet_payloads.rs`) and a menu step, run
right after `compose` and before anything is minted or printed: for every
product `Document::carried_services` names, it lists what the fleet repository
holds, newest first, and the operator picks — a bare Return takes the newest,
never chosen silently, matching R-176 Q6's own words. The choice becomes
`[carry.<product>] version = "…"` in the document. A required product with
nothing published is refused right there, naming `cargo rivet fleet publish`,
before the certificate is minted or anything is written. Which repository to
read is `RIVET_FLEET_REPOSITORY`, set by `cargo rivet provision`'s orchestrator
to the same root it commits into, so an orchestrated run's two halves never
disagree; run standalone, the interview falls back to the same default root
every other fleet tool resolves.

**P4/Q10 — recording moves out of the interview, to after the write.**
`cmd/rivet-provision`'s `w` key no longer calls `rivetos_fleet_repo::
record_document` at all — `record_in_fleet_repository` and its two tests were
deleted outright. `w` now only composes, writes the document and the staging
bundle locally, and writes the breadcrumb `cargo rivet provision` reads — the
same three things it always did, minus the fleet-repository copy. That copy is
now made by `cargo rivet provision` itself (`cmd/cargo-rivet/src/provision.rs`,
a new `RealFleetRecorder`), called only once `write-image` has returned `Ok`
**on a route that targeted a device** (`--device PATH` or `confirm` —
`write_image::run`'s own condition for "wrote a card" rather than merely
"prepared an image", restated here so nothing is committed for the latter). A
declined or failed write, or a run that only prepares an image, commits
nothing — exactly R-174's "a partial result is a failure," applied to the
record as much as to the card.

**Q7 — enrolment.** The same `RealFleetRecorder` also enrols a server or
member into the OPERATOR's register (`crate::device::default_register_dir`,
the same one `cargo rivet run <device>` already resolves) — a `Device` record
built the way `cmd/rivet-fleet`'s `add_device` builds one, except its
identifier is derived from the machine's own control-plane pin
(`rivet_core::identifier::from_entropy`, `ADR-049` §8.1) rather than minted at
random, so re-provisioning the same board reaches for the same record rather
than a second one. A stand-alone document enrols nothing. **The duplicate
check — an id or a hostname already in the register — runs BEFORE the write**,
a new step in the sequence (§4.5–4.12 step 4), not folded into the commit
afterward. The pin itself is a workstation fact ("Identity") the document
never carries, so it travels from the interview to the orchestrator on a new,
best-effort second line of the existing breadcrumb file
(`pin=<64 hex characters>`) — or, on the `--document PATH` route, which runs
no interview and produces no breadcrumb at all, from two new flags,
`--identity PATH`/`--pin HEX`, taken before `write-image`'s own parser ever
sees argv, on `--staging`'s own footing.

**What this means for §4.5–4.12.** The section now describes six steps
instead of four: preflight, interview (which itself now asks about
versions), staging check, the new enrolment-duplicate check, the write, and —
new — recording. `SPEC-05` §8's "enrolling a fleet member" bullet is narrowed
to what is actually still out of scope: adopting a member that already
exists and already booted, not pre-enrolling one during its own
provisioning, which this item builds.

**Verified by:** `cargo test -p rivetos-provision` (95 passed) —
`versions::list`'s newest-first ordering and its "nothing published"/"never
built" empty-list cases; `carried_version_from_answer`'s newest-preselected,
pick-an-older-one and refused-answer cases; `resolve_carry`'s own refusals
(no repository at all; a required product unpublished) and its no-op path for
a document that carries nothing. `cargo test -p cargo-rivet --bin cargo-rivet`
(439 passed) — `run_with`'s ordering (the fleet recorder's `plan` runs before
the writer and its `commit` only after a successful, device-targeting write;
a declined or failed write, or one with no device targeted, commits nothing;
a duplicate `plan` refusal stops the sequence before the writer is ever
reached; an explicit pin wins over the interview's own); `role_of` and
`enrolment_device` against fixture documents; and, against REAL
`rivetos_fleet_repo`/`rivetos_register` pointed at scratch directories never
the maintainer's own — `RealFleetRecorder::plan`/`commit`'s own cases
(a fresh enrolment, a duplicate by id, a duplicate by hostname, a stand-alone
document touching neither) and a `--document PATH` scratch end-to-end proving
the document lands committed and the register holds the enrolled record
afterward, with a second run under the same pin refused as a duplicate before
anything is written. `cargo clippy -p rivetos-provision --all-targets` and
`cargo clippy -p cargo-rivet --all-targets` — no new warning. **Unproven, and
unchanged by any of this**: the interactive interview's own version-selection
menu, in a real terminal — R-178 P6's own item, with a Rust `portable-pty`
harness, rather than this one's — and, as always, whether a card produced
this way boots, which §7 already records as unmeasured.

### 2026-09-13 — R-175 and R-176 Q11 revert the build-per-run orchestrator

The entry directly below this one describes R-174's design: `cargo rivet
provision` cross-building, signing and publishing a role's own
`rivet-fleetd`/`rivet-proxyd`, pinning the version into the document, and
building a whole card image, on every run. Told that shape, the maintainer
named it wrong (`.rulings/` R-175): software is built and signed once,
published into the fleet repository, and provisioning is a consumer of that
repository, never a producer. It should never need to compile or sign
anything of its own — that is what the repository is for. A follow-up review
(`.rulings/` R-176) walked an eight-step model of how the command is meant to
run and confirmed the same thing from a different angle: a release puts a
system image and every application into the repository, signed, before any
provisioning; provisioning selects from what is there. Q11 ruled the remedy
plainly: revert the build-per-run code; keep the fixes made alongside it.

**What was removed.** `cmd/cargo-rivet/src/payloads.rs` (the cross-build,
package, sign and publish stage, and the `[carry.<product>] version` pin) and
`cmd/cargo-rivet/src/card_build.rs` (running `tools/container/build-card.sh`
with every input resolved by the tool) are both gone, along with the Docker
check and the silent `fleet::init` fleet-repository creation `provision.rs`'s
preflight ran before the interview. (`.rulings/` R-176 Q9 answers the fleet
repository question a different way for later: a missing fleet or CA should
be *offered* to the operator as a guided detour, not created without asking —
offering it is not built either, so for now preflight simply has nothing left
that needs a fleet repository at all.)

**What was kept**, because R-176 Q11 asked for exactly that: the credentials
preflight (narrowed to credentials alone, since nothing after it builds or
publishes); the staging-bundle refusal, moved out of `payloads.rs` into
`provision.rs` itself and wired through a small `StagingCheck` trait so
`provision_tests.rs` can still fake it — it catches a hand-composed
`--document PATH` that would otherwise silently write a server or member card
with no credentials for its own application service; the interview-launch
fix (`resolve_interview_source`) that finds `cmd/rivet-provision` from a real
install rather than only from a `target/debug`-shaped directory — R-191 and
R-192 later made that the ordinary route by installing both binaries, and
§4.5–4.12's own step 3 is the current account of it; and `progress.rs`'s stage
announcements, now narrated for the one stage that remains.

**What this means for §4.5–4.12.** The section above now describes exactly
four steps — preflight, interview, staging check, write — and an image comes
from `--image PATH` or `write-image`'s own menu, the same as before R-174
touched this command at all. Selecting a prebuilt, signed image from the
fleet repository, rather than requiring one named by hand, is real work
`.rulings/` R-176 describes but does not build; `ADR-052` §7.2.3 already
records that the repository holds no whole card images today, which is why
that work has to land before this command can pick one.

**Verified by:** `cargo test -p cargo-rivet --bin cargo-rivet` (408 passed) —
every sequencing case in `provision_tests.rs` against fakes, and
`require_staging`'s own refusal and success cases against real scratch
files. `cargo clippy -p cargo-rivet --all-targets` — no new warning.
`cargo doc -p cargo-rivet --no-deps` — no warning. **Unchanged by any of
this**: whether a card built this way boots — §7 is still that honest
account.

### 2026-09-13 — R-174 reverses R-173: the orchestrator builds the card, not only writes it

The provisioning dry run this ruling is named after followed README and this
document exactly and produced a fleet-server card with no `rivet-fleetd` and
no register. The gap was not a missing feature so much as three of them
stacked: getting the role's own application services onto the card still
meant cross-building, stripping, packaging, signing and publishing
`rivet-fleetd`/`rivet-proxyd` by hand (blocker B1) and pinning the version
into the document by hand (B3); and building the card still meant
hand-typing `RIVET_DOCUMENT`, `RIVET_STAGING`, `RIVET_IDENTITY` and
`RIVET_FLEET_REPOSITORY` into `tools/container/build-card.sh` (B2). The
maintainer's requirement, stated at the top of this document, ruled
that every one of those is the tool's job.

**What changed.** `cmd/cargo-rivet/src/payloads.rs` is new: it reads the
composed document's own `carried_services`, and for every one this tree owns
a `cmd/<product>` crate for, cross-builds it inside the container
(`tools/container/exec.sh`, the same recipe
`skills/deploying-an-application/SKILL.md` already documented by hand),
packages and publishes it **in this same process** through the existing
`cargo rivet package`/`cargo rivet fleet publish` machinery
(`crate::package_to`, `fleet::publish_verified`), and rewrites the document
with a `[carry.<product>] version` pin equal to exactly the version that
build produced. `cmd/cargo-rivet/src/card_build.rs` is new too: it runs
`tools/container/build-card.sh` with the document, the `ADR-049` staging
bundle, the identity directory and the fleet repository all passed as
arguments the tool already resolved, into an output directory it names and
then scans itself — closing a `RIVET_OUT` shadowing trap where a caller's own
output directory could silently disagree with `write-image`'s hard-coded
`./out` scan. `cmd/cargo-rivet/src/provision.rs` sequences both ahead of the
existing hand-off to `write-image`, plus a new preflight (credentials,
Docker, the fleet repository) that runs before the interview rather than
after it.

**Two decisions this pass did not make unilaterally, named rather than
guessed.** First: whether provisioning should itself run
`tools/card/bump.sh` (which increments this checkout's own workspace version
and commits that change) so that every build gets a fresh, non-`-dirty`
version stamp with no operator step at all. This pass did **not** automate
that — `card_build.rs` reads the current tree's `git describe` read-only and
refuses a dirty tree, and a maintainer who wants a new stamp still runs
`bump.sh` himself, exactly as before. Automating a version bump inside a
provisioning run is a bigger decision than sequencing the existing steps, and
is recorded here for the maintainer to rule on rather than assumed. Second:
which identity directory a hand-typed `--document PATH` (skipping the
interview) should be built against, when the operator answered the
interview's own *Identity* question with something other than `cargo rivet
setup`'s conventional path — the orchestrator uses the conventional path
unconditionally, and there is no flag yet to name a different one for this
specific case.

Also fixed in the same pass, all three named in this document already:
`cmd/rivet-image/src/main.rs:1407`'s stale claim that `rivet apply install`
takes a payload PATH (it takes a digest since `ADR-052` §7);
`tools/container/build-card.sh`'s `under_repo`/`repo_relative`, which compared
a Windows-style key path against a POSIX `$REPO` byte for byte and never
matched even when the path genuinely was under the repository; and
`tools/card/build.sh`'s `verify_carried_payloads`, which skipped verification
entirely for a card whose payloads came only from `--fleet-repository`
because it guarded itself on `RIVET_APPLICATIONS`'s own count alone — it now
reads the true count back from `cmd/rivet-image`'s own report, which already
lists every payload it verified and wrote regardless of source.

**Verified by:** `cargo test -p cargo-rivet` (421 tests) — every new refusal
and every sequencing case in `provision_tests.rs`, `payloads_tests.rs` and
`card_build.rs`'s own tests, all against fakes: preflight runs before the
interview and before the card pipeline; a server or member document with no
staging bundle is refused before a byte is built; a build or publish failure
for one product stops the sequence before the document is rewritten and
before a second product is even attempted; the pin written is always exactly
the version the (fake) build reported. `cargo clippy -p cargo-rivet
--all-targets` — no new warning. `uv run tools/source/prose.py --check` and
`uv run tools/docs/claims.py --check` — both pass. **Unproven by any of
that**: a real Docker build of `rivet-fleetd`/`rivet-proxyd`, a real publish
into a real fleet repository, and a real `tools/container/build-card.sh` run
driven by this orchestrator — see this change's own report for what an
end-to-end run into scratch, with fictional answers and no terminal, through
`--document`, actually measured; and, unchanged by any of this, whether a
card built this way boots — §7 is still that honest account.

### 2026-09-09 — `rivet-fleet`'s own composer catches up to most of §5, and where it still cannot

Measured the day before a brand-new fleet was minted:
`rivet-fleet export provisioning --device rivet-01` — the recovery path for a
machine whose card must be rebuilt — composed a document missing a resolver,
a timezone, `[applications.rivet-proxyd] upstream-name`, a control plane
bound to every interface, and a service (`reliquary-rivet`) the machine
actually runs. `cmd/rivet-provision` already answered every one of §5's
bullets; `cmd/rivet-fleet/src/compose.rs` (`ADR-011`'s second implementation
of one job) answered none of them, because nothing in the register held the
facts and nothing applied the role's defaults.

**What changed.** `rivetos_register::Device` gained three fields —
`fleet_role` (`server`/`member`/`stand-alone`, `rivet-provision`'s own
vocabulary, not the free-text `role` label), `timezone`, and `applications`
(per-application settings, keyed by application then by key). `compose.rs`
gained a role-default layer beneath the fleet's and the device's own for
`[control-plane] listen` and for services, and split into two composers:
`provisioning` stays permissive so `add document`'s adoption can keep working
against a record §5 would refuse, and `provisioning_complete` — used by an
export, a card build, and a live write, and only those — adds §5's own gates
and refuses, naming what is missing, rather than producing a document that
boots silently wrong. `add document`'s `adopt` gained arms for `timezone` and
for any `[applications.<name>].<key>`, so a machine that already answers
these can have the register catch up to it. `set fleet-role` (and
`add device --fleet-role`) let an operator answer the role directly.

**The service gap had no code defect behind it, checked rather than
assumed.** `rivet-fleet add document --device rivet-01` (read-only, no
`--confirm`) already listed `services.reliquary-rivet.enabled` as adoptable
before any of this landed — the generic `["services", service, "enabled"]`
arm takes any service's name without needing it known in advance. The register
was simply stale: nobody had re-run adoption since Reliquary was deployed.

**What is still open, named rather than guessed past.** `kiosk.auto-run` is
still not a register field — the same dry run still lists it as `CANNOT` —
because §5.1 is a kiosk machine's addendum and was out of this pass's scope.
There is no `set timezone` verb, so a device whose document has not already
answered it has no way to be given one except through adoption. And
`rivet-01`'s own register record still carries `listen = "192.168.50.10:7777"`
— an address, not `0.0.0.0:7777` — recorded before this pass and left alone
on purpose: the register is the authority and an agent does not overwrite an
operator's own recorded value on the strength of a mismatch with the machine,
which is the asymmetry `ADR-034` §6.2 already rules. `rivet-fleet set listen
--device rivet-01 --listen 0.0.0.0:7777 --confirm` is the operator's own
repair.

**What unifying the two composers would take**, asked for and not done here:
`cmd/rivet-provision`'s `services_for`/`listen_for`/`upstream_name_for` and
`cmd/rivet-fleet/src/compose.rs`'s `role_services`/`role_listen_default` are
now two copies of one small function each, verified to agree by construction
rather than by a shared implementation. Unifying them needs a library target
on `rivetos-provision` — it has none today, only a `[[bin]]` — or a third,
small crate the two `cmd/*` binaries both depend on, carrying just the role
vocabulary and its two defaults (a few dozen lines). `upstream_name_for`
itself should NOT move: it reads `~/.rivet/ca/root.crt`, and `rivet-fleet`'s
register deliberately holds no path to a certificate authority (`ADR-034`
§3) — unifying that half would mean deciding whether the register-side tool
is allowed to read a workstation's CA at all, which is a bigger question than
this pass was asked to answer. The right split is a shared *role defaults*
crate and two composers that still each own their own document assembly and
their own I/O.

**Verified by:** `cargo test -p rivetos-fleet` (`compose::tests`,
`main::tests`) — every role default (server binds every interface and turns
on its three services; a member binds and turns on `rivet-mdnsd` alone; a
stand-alone or roleless device gets neither and writes byte-identical
documents to before); the device layer beating the fleet layer beating the
role layer, including an explicit `false`; `upstream-name`'s refusal firing
whether the proxy is on by an explicit service or by the role, and not firing
when the proxy is off; both refusal reasons named together; adopting
`timezone` and `[applications.*]` under more than one application; and a
fully-adopted record composing a FINAL document end to end. `cargo test -p
rivetos-register` — the three fields round-tripping, and absent still
composing byte-identical documents. `cargo clippy --all-targets` — identical
findings before and after, on both crates. `uv run tools/docs/claims.py
--check` and `uv run tools/source/prose.py --check` — no new finding beyond
what was already pinned or already spread. **Measured live, read-only,
against `rivet-01` and the real register at `%APPDATA%\rivet-fleet\register`**
— the acceptance-test command above, and the `add document` dry run this
entry's numbers are drawn from. **Unproven:** an `apply document` or a card
build actually reaching a board with a complete record — `AGENTS.md` §9's
"do not touch the maintainer's live instance" and this task's own scope kept
every write to the register and to the machine out of this pass, on purpose.

### 2026-09-09 — the three gaps the previous entry named rather than closed

Closed the day before the same brand-new fleet was minted, left for this
second pass on purpose: **`set timezone` did not exist**, so a device record
minted before its machine had ever booted had no way to answer `SPEC-05` §5's
timezone gate; **`kiosk.auto-run` had no register field at all**, so a kiosk
machine's record — `rivet-01`'s own, running `aura-renderer` — could not be
fully rebuilt from the register even by adoption; and **`add document`'s plan
report never called `diff::changes`**, so a real difference between the
register and a running machine could sit beside a genuine loss and never be
shown. The third is why the maintainer never saw that his own register held
`[control-plane] listen = "192.168.50.10:7777"` while `rivet-01` bound
`0.0.0.0:7777` — a dry run naming only losses does not surface a change.

**What changed.** `rivetos_register::Device` gained `kiosk_auto_run:
Option<String>`, on `timezone`'s terms exactly: absent composes no `[kiosk]`
table, `add document`'s `adopt` gained a `["kiosk", "auto-run"]` arm, and
`crate::compose::render` (`cmd/rivet-fleet`) writes it back out as
`[kiosk] auto-run = "…"`. Two new register-only `set` verbs answer the gap
adoption cannot reach: `set timezone --device <name> --timezone <zone>`,
checked against `rivet_core::timezone::Timezone` — the grammar
`rivet-config`'s own parser enforces, stated as not reaching existence
because `cmd/rivet-provision`'s zone database check lives in a binary crate
with no library target this tool can call — and `set kiosk-auto-run --device
<name> --application <name>`, which checks nothing beyond non-empty because
`rivet-config`'s own parser does not either. `add_document`'s report-building
was split out of the network-reaching wrapper into `adopt_report`, testable
without a fake transport, and both its PLAN and its confirmed report now call
`diff::changes` beside the losses — a change is never adopted by this verb,
but it is reported every time one is found, so `apply document`'s own effect
is visible before it runs rather than after.

**`ADR-034` §6.1's both-or-neither does not bind either new verb**, and the
previous entry's own guess that it would bind `timezone` was wrong: that rule
is about a value that is *how the tool reaches the machine*, and neither a
timezone nor a kiosk application is that. Both compose like `fleet-role` and
`pin` already do.

**What is still open, and is named rather than closed.** `SPEC-05` §5.1's own
gap — nothing at card time can know whether a named kiosk application will be
installed — is unchanged; `set kiosk-auto-run` records a name and validates
nothing about it, on purpose. Whether an IANA zone actually exists is checked
only where a machine has already answered it (`add document`'s adoption);
`set timezone` checks grammar and says so in its own refusal. And
`rivet-01`'s real register is untouched by this pass, by the same instruction
as before — its `listen` mismatch and its missing `kiosk.auto-run` are still
there for the maintainer's own `add document --confirm` to resolve.

**Verified by:** `cargo test -p rivetos-fleet` (`main::tests`,
`compose::tests`) — `set timezone`'s success and its refusal on a
non-zone-shaped value; `set kiosk-auto-run`'s success and its refusal on an
empty name; adopting `kiosk.auto-run` landing on the device record and
nowhere else; the composer emitting `[kiosk] auto-run` when the record names
one and nothing when it does not; and `add document`'s report — both its PLAN
and its confirmed write — naming a loss and a change together, constructed
to the same shape the real register/board mismatch had. `cargo test -p
rivetos-register` — `kiosk_auto_run` round-tripping and composing no table
when absent. `cargo clippy --all-targets` — identical findings before and
after, diffed with `git stash` rather than eyeballed, on both crates. `uv run
tools/docs/claims.py --check` — 0 new, 3 spread, matching the state before
this pass; the `deep-citations` and `rot-markers` ratchets were watched
individually to confirm this pass added neither a dotted citation nor a
fourth spread. **Unproven:** an `apply document` or a card build reaching a
board with either new field recorded — every write to `rivet-01` and to the
maintainer's real register stayed out of this pass, on purpose, and the
acceptance command
(`export provisioning --device rivet-01`) still refuses, exactly as it did
before, because running `add document` himself is the maintainer's own to do.

### 2026-09-09 — the authority can be backed up and restored, and `rivet-reset` checks for one

The day before a brand-new CA and a brand-new fleet were minted, an audit
found the gap this closes: nothing in `rivet-provision`'s `ca` module or
`cargo rivet setup` copied the authority anywhere, nothing read one back, and
step 1 (§4.1) destroyed `root.key` — `remove_file` on each authority file,
then `remove_dir` — with no check that a copy existed. Losing it orphans
every certificate the fleet's machines carry, with no way to issue another.

`rivet-provision ca backup <directory>` and `ca restore <directory>` close
it (§4.0, §2.6). Two decisions worth recording:

* **`restore` is the inverse of `rivet-reset`, not of a plain copy.** A
  restore that only moved files back would leave the browser in exactly the
  state `rivet-reset` produces — the files present, trusted by nobody, and
  the one browser certificate that would work invisible among whatever the
  store still holds. So `restore` re-installs `root.crt` into the Root store
  and `browser.pfx` into Personal, verifying the private key came in with it
  the same way §4.4 already requires of a fresh mint — a return code is not
  evidence, because `certutil -importpfx` reports success while leaving a
  key unattached when a copy of the certificate is already in the store.
* **`rivet-reset`'s backup check compares `root.crt` bytes, and does not open
  a key.** `--backup <directory>` names where to look; the match is
  byte-for-byte against that directory and one level of its subdirectories,
  which is the honest version of "is there a backup" without parsing or
  verifying anything this program has no business opening. When none is
  found, `--proceed-without-backup` is the one extra word required — not a
  wall, because the maintainer's response to people who skip the
  instructions and lose their keys is a shrug (`.rulings/` R-316).

Both verbs share one file-copy engine (`cmd/rivet-provision/src/backup.rs`)
because the refusal is identical in both directions: neither a backup nor a
restore silently overwrites what is already at its destination.

**Verified by:** `cargo test -p rivetos-provision` (`backup::tests`) —
the missing- and empty-source refusals, every file named and sized, the
overwrite refusal and what it names, `--replace` overriding it, and a full
mint-backup-delete-restore round trip asserting every byte survived, all
against temporary directories; `cargo test -p rivetos-reset`
(`workstation::tests::a_backup_is_matched_on_the_exact_bytes_of_root_crt` and
`…_one_level_down_is_also_found`) — the byte matcher finding a real backup,
refusing a different authority's, and checking one level of subdirectories.
**Unproven:** `restore`'s re-installation into a real Windows certificate
store — every test run against `--no-install`, per `AGENTS.md` §9's
prohibition on an agent writing to the maintainer's store — and the whole
ceremony run by a person rather than exercised through unit tests.

### 2026-09-09 — the retyped identifier is replaced by `[y/N]` and an elevated re-check; the write moves to a second process

Ruled by the maintainer the same day §4.10–§4.12 first landed, after
`AGENTS.md` §10.4's write-path was measured against a real Windows host rather
than assumed: an unelevated shell was refused even a *read* of
`\\.\PHYSICALDRIVE2`, so `write_bytes_and_verify`'s `OpenOptions::new().write(true)`
against a physical drive was never going to succeed from the unelevated
process `cargo rivet provision` already, correctly, ran as.

Two changes followed from that one measurement:

* **The retyped-identifier confirmation is gone, and `Write it? [y/N]` (default
  no) replaced it.** The retyped identifier was reasoned about as the guard
  against a wrong disk; it never actually was one, because the unelevated
  parent can be wrong about which disk is which — a stale enumeration, a card
  swapped for another during the confirmation pause — and no amount of typing
  or clicking in that same process changes what it believes. The guard that
  actually catches that moved to the elevated child re-enumerating on its own
  account, which is the one check that cannot be clicked through because it is
  not a prompt at all.
* **The write moved into a second, short-lived, elevated process** —
  `cargo-rivet` itself, under `provision --elevated-write`, launched from
  `current_exe()` and relaunched by PowerShell's `Start-Process -Verb RunAs`.
  Handles do not cross that boundary, so progress and the result travel as
  files rather than a pipe. `cmd/cargo-rivet` gained no `windows-sys`
  dependency: `Start-Process`, `Set-Disk -IsOffline` and `Get-Disk` are all
  shelled out to as PowerShell, the same posture `crate::enumerate` already
  took for CIM.

**The unelevated boot-partition check and `--verify-only` were added in the
same change, not asked for by the original requirement.** Once the write had
to happen in a process the parent could not see into, a second, independent
confirmation that needed no elevation at all became worth having for free —
volume handles were already known to open unelevated from the same
measurement that started this entry — and answers *"will this card boot
configured,"* the failure that cost a whole bring-up on 2026-09-07.

**Verified by:** `cargo test -p cargo-rivet` — the declined-UAC path
interpreted as nothing-written rather than an error; the elevated child's own
re-enumeration refusing a real non-removable disk on this development host
(read-only, `AGENTS.md` §9); the task file round-tripping; the progress file
parsing correctly including a torn final line; every result-file outcome
round-tripping; and the write-verify-hash machinery itself, end to end,
against a temporary file standing in for a device. **Unproven**: an actual
elevated write against a real card, the real UAC prompt and its cancellation,
and `Set-Disk -IsOffline` against a real disk — `AGENTS.md` §9 forbids an agent
from writing to a real block device, so the maintainer performs the one real
write himself.

### 2026-09-09 — §4.10, §4.11 and §4.12 built

`PLAN-Provisioning-2026-09-07.md` §4 (Wave 4). §4.10 read:

> **It MUST enumerate the removable devices, describe each one well enough to
> recognise it — size, model, letter — and ask.**
>
> **It MUST NOT guess, default to one, or accept a path without
> confirmation.** Writing an image to the wrong device destroys it.

and the module header comment above `cmd/cargo-rivet/src/provision.rs`'s
`write_device` function already claimed *"it never writes a device it has not
been told is removable"* — which was false until this landed, and a false
claim beside a destructive operation is worse than none.

Two things the requirement did not anticipate:

* **Enumeration needed a JSON parser, and the workspace deliberately carries
  none.** `Get-CimInstance ... | ConvertTo-Json` was chosen over
  `cmd/rivet-reset/src/store.rs`'s space-delimited convention because a disk
  `Model` string routinely contains spaces, so `cmd/cargo-rivet/src/json.rs`
  is a closed, hand-written parser scoped to exactly this producer — the same
  posture `cmd/rivet/Cargo.toml` records for the JSON this project writes,
  pointed at reading instead.
* **`confirm` was already the destructive word, and turned out not to be
  enough on its own.** `AGENTS.md` §10.4 names *naming the device* as the
  guard, not the confirmation word — so a second, independent step was added:
  typing the chosen disk's own identifier back before anything opens. A menu
  number is how the disk is chosen; the retyped identifier is how it is
  confirmed, and the two together catch what `confirm` alone cannot — a slip
  on the menu, or a mistyped `--device`.

**Verified by:** `cargo test -p cargo-rivet` — unit tests against a temporary
FILE standing in for a device (never a real block device), covering a
non-removable disk being refused with no override, a wrong retyped identifier
being refused before anything opens, a short write and a digest mismatch each
being reported as the card being NOT usable, and the JSON parser against a
real `Win32_DiskDrive` payload captured from this host, empty case included.
**The real enumeration was also run, read-only, on the development host** —
it correctly named two fixed internal disks and one attached USB device as
removable, with its four mounted drive letters. **Writing to a device and the
`/sys/block` path remain unproven** — no agent may write to a real device
(`AGENTS.md` §9), and this project's development host has no `/sys` at all
(`AGENTS.md` §10.5) — the Linux reader is proven only against a scratch
directory standing in for `/sys/block`.

### 2026-09-07 — §4.9 and §5's last three bullets built; §7 given a measurement it does not have

Wave 3. `ADR-049` is the decision and carries the reasoning; what changed here
is what the requirements say about themselves.

**§4.9 said only what the card must carry**, and left the shape of it open:

> **It MUST be written in `rivetos_register`'s on-disk layout** —
> `register.toml` beside a `devices/` directory — into `rivet-fleetd`'s data
> directory, **owned by that application**, mode as the supervisor would create
> it.

That is still true and is still the body. What the requirement did not
anticipate, and what the build found, is that **nothing that could write the
card could satisfy the ownership clause**: the uid is derived by the supervisor
at discovery, so delivery had to become a third hop rather than a card-time
write. The register also turned out not to be a special case — the same hop
carries five credential files and the proxy's three — so the requirement is met
by a general mechanism and §4.9 now says so.

**§5's `upstream-name` bullet said `NOT BUILT`** and called its own absence
*"the same defect this document is about"*, because §10 had recorded the setting
being fixed by hand and §5 had listed the service without it. It is built, and
the stronger form was taken: composing a document that enables the proxy and
omits the setting is now impossible rather than merely discouraged.

**§7 acquired a paragraph saying it has not been measured**, which is the
opposite of a status upgrade and belongs at the top of the section rather than
in a note. The specification is an acceptance test; the honest reading on
2026-09-07 is that the card now carries what §10 says a hand had to install, and
that no machine has booted from one.

### 2026-09-07 — §4.5, §5's first two bullets, §5.0 and §5.1 built

The requirements were written against a menu that composed four lines and asked
six questions. They now describe what it does. Three things the requirements did
not anticipate, each of which changed the answer rather than the wording:

* **The schema's loopback default is CORRECT and had to stay.** §5 read *"the
  current default of `127.0.0.1:7777`"* as one thing to be reversed. It is two:
  the value a **role** composes, which was the defect, and the value that
  applies when a document says **nothing**, which is `ADR-018` §8's fallback for
  a machine whose stored document cannot be read. Changing the constant would
  have made a machine start answering the network because a file went missing.
  `ADR-004` §6.2.1 is the decision.
* **A reachable listener is only correct on a card that carries an identity.**
  `cmd/rivet-mcpd` refuses a non-loopback address when it was given no TLS
  identity, before it binds — so `0.0.0.0:7777` on an identity-less card is a
  machine with *no* control plane rather than an unreachable one. That is why
  `stand-alone` keeps loopback and why the tool prints the coupling.
* **`[control-plane]` is written out even when its value is the default.** An
  absent table cannot be repaired with `apply edit`, for the same reason §5.0
  gives about `timezone`: appending to a document that ends in a table puts the
  keys inside that table. The requirement asked for a value; what a person needs
  is a key that exists.

**§5.0 gained the check nothing in this workspace could make**, and it is the
one that closes the silent failure rather than describing it: a well-formed name
for a zone the image has no file for. See §5.0.

### 2026-09-07 — §2, §9.2 and §11.2 built

**§2, `rivet-reset`.** The section was a requirement against a program that did
not exist; it now describes `cmd/rivet-reset`, and §2.5 was added to say plainly
which half of it is proven. The requirement anticipated everything except two
things the building found:

* **The key-matching rule had to become normative text, not an implementation
  note.** §2.1 said only that a re-minted authority "may carry the same subject
  name", leaving *how* to tell them apart to whoever wrote the code — and the
  natural thing to write is a name comparison, which is the defect. It is now
  §2.3.
* **A certificate wearing the name that the key does not verify is a THIRD
  answer.** The original text implied two — ours, and everyone else's — and
  under that reading the retired authority's own leaves are either deleted
  silently or invisible. They are now reported and left alone by default.

**§9.2, `rivet-fleet export card-plan` emits wrong instructions.** It read:

> `rivet-fleet get log` does not exist (`get` has `device` and `status`); the
> card build is routed through WSL and `./tools/card/build.sh`, superseded by
> `ADR-026` §7's `tools/container/build-card.sh`; and a step cites "the identity
> from step 3" when it is step 2.
>
> **This is the closest thing to §3's script that exists**, which is why its
> wrongness matters more than its size.

Step 6 now names `cargo rivet run … -- get log previous all` for the journal and
`rivet-fleet get status --device <name>` for the supervisor's answer, and says
in the plan itself that this tool has no `get log`. Step 4 names
`tools/container/build-card.sh`. Every reference to the identity says step 2.
`cmd/rivet-fleet/src/card.rs`'s test asserts all three, including the negatives —
a template that drifts once drifts again, and the test is what notices.

**§11.2, `cargo rivet provision --document` injects an unvalidated document.**
It read:

> `cmd/cargo-rivet/src/provision.rs` reads the file as bytes and writes it to
> both boot partitions. **Nothing parses it**, so a document with a typo in it
> becomes a card that refuses on first boot — which is the exact end
> `tools/card/provisioning.example.toml` names as the wrong one, and the reason
> §4.7 makes `rivet-provision` do the checking. It should either parse the file
> or say plainly that it did not.
>
> **It also replaces whatever `rivet.toml` the image already carried, without
> saying so.**

It parses the file with `rivet_config::Document::parse` — the same parser the
board refuses with — before the image is opened, refuses with the schema's own
words, and reports the hostname, interface count, service count and effective
control-plane listener rather than a tick. It now says that the document
replaces any the image was built with.

### 2026-09-07 — §4.6, §4.7, §9.1 and §9.3 built

The four repairs `PLAN-Provisioning-2026-09-07.md` numbers 1.1, 1.3, 2.1 and 2.3.
The superseded text, and why each changed:

**§9.1, `rivet-fleet add identity` ignores `--confirm`.** It read:

> Its help states the flag is what makes it act and that without it *"the plan
> is printed and nothing changes."* `add_identity` never reads
> `request.confirmed`. The dry run mints the seed and writes `device.seed`,
> `device.crt` and `peers`; the subsequent real run fails, because `cargo rivet
> identity new` correctly refuses to overwrite a key that may exist only once.
>
> **This is the worst flag in the toolset to be wrong about**, because the
> artifact is a private key that exists once and the safe habit destroys it.
> **MUST be fixed before §4 is walked.**

The unconfirmed path now names every file it would write, says which of them
already exist, says whether the register would replace a public half it holds,
and returns having touched nothing.

**§9.3 and §2.4, `rivet-provision ca` always writes to the real certificate
store.** §2.4 read:

> `rivet-provision ca` writes into the real Windows certificate store
> unconditionally — proven 2026-09-07 by removing `certutil` from `PATH`, which
> Windows resolves from the system directory regardless. **So there is no dry
> run of §4 today, and every rehearsal degrades the store it is rehearsing
> for.** `rivet-reset` is what makes practice survivable, which is why it is §2
> and not an appendix.

The mechanism was right and the conclusion no longer follows: `--no-install`
skips `browser::place` and writes the bundle anyway. `rivet-reset` is still §2
because a store that has already been dirtied is not something a flag can clean,
and because §4.1 is a step in the twelve regardless.

**§4.6 and §4.7** were requirements against a `w` that printed and exited; they
are now descriptions of what it does. The one thing the requirement did not
anticipate is that `~/.rivet/provision` already held two kinds of file under one
naming scheme — an answers notebook and a hand-written document — so the written
document takes `.provisioning.toml` rather than colliding with `s`.
