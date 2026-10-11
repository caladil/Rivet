# Completing `rivet-provision` — a brief ahead of the design

* **Status:** **Brief.** Commissioned 2026-09-07, not investigated, not ruled
* **Date:** 2026-09-07
* **Target Audience:** the next agent to pick this up, and Human Maintainers
* **Prerequisite reading:** `docs/design-candidates/Fleet-Bring-Up-Findings.md`
  — §3 of that document is what provoked this one

---

## 1. Why this exists

Bringing up `rivet-01` on 2026-09-07 required an agent to hand-write the
machine's TOML, hand-mint the authority, hand-build a PKCS#12, hand-import a
certificate, and hand-append four `[services.*]` tables over the control plane
after the card had already booted. **The maintainer's razor, ruled that day:**
every time he could not do something and the agent had to do it for him was,
frankly, the system failing. *(`.rulings/` R-301.)*

By that rule the provisioning tool failed at almost every step, and the reason
is not that it is broken. It is that **it stops less than half way** and nobody
had written down where the line was.

## 2. The maintainer's questions

These are the spine of the work. Answer each one **in the record**, not in a
chat reply. *(`.rulings/` R-302.)*

1. **Why did it stop at the `w` step instead of writing to disk?**
2. **Why was it left incomplete?**
3. **Why does it not provision the services?**
4. **Why was this not thought of sooner?**
5. **Why does it lack the logic to ask which removable USB device he wants to
   write to?**
6. **Why does it not write yet?**

Question 4 is not rhetorical and must not be skipped. It is a question about how
this project decides something is finished, and the answer belongs in the
findings record even if it is uncomfortable.

## 3. What is already known — verified 2026-09-07, so do not re-derive it

**`w` does not write anything.** `cmd/rivet-provision/src/main.rs:394`. The
branch:

* refuses if any field is invalid,
* calls `compose(&fields)`,
* mints the fleet server's certificate when `Fleet == "server"`, via
  `mint_for_host(hostname)` — with the comment that this is the first moment the
  hostname exists, which is correct and worth preserving,
* **`println!("\n{doc}")` — prints the document to stdout,**
* returns `ExitCode::SUCCESS`.

There is no file write, no device enumeration, no card image, no register
record. **The letter `w` promises a write the code never performs**, and a
person who types it reasonably concludes the card was written. That is the
whole of finding 1 and it is confirmed.

**The composed document carries no `[services.*]` tables.** On 2026-09-07 the
operator had to append `rivet-mdnsd`, `rivet-proxyd`, `rivet-bluetoothd` and a
`[kiosk]` table by hand — three of them over the control plane, after boot,
because they were not on the card. `[control-plane] listen` was also absent and
defaulted to `127.0.0.1:7777`, which produced **an unreachable board on first
boot** and cost a full rebuild-and-rewrite cycle.

**`enrol` is about the browser certificate, not the device.** `main.rs:773`.
Nothing in this tool writes a device record into a register, despite
`cmd/rivet-fleetd/src/fleet.rs` stating plainly that the fleet server's first
record is written **onto the card by `rivet-provision`**, *"because at that
moment there is no server listening to be told."* That write does not exist.
Its absence is why `rivet-01` serves an empty register today.

**`ca server <name>` takes a literal name list** and overwrote a three-suffix
leaf with a single-SAN one. The authority already records its suffixes in
`~/.rivet/ca/suffix`.

## 4. The shape of the work — a proposal to argue with, not a plan to execute

**Do not start coding from this list.** It is here so the investigation has
something to disagree with.

* **`w` becomes what it says**, or is renamed. Probably: write the document to
  `~/.rivet/provision/<hostname>.toml` and report the path.
* **A card step that enumerates removable devices** and asks. `cmd/rivet/`
  already has block-device code (`src/device.rs`) — find out whether it can be
  reused rather than written twice, and whether `cargo rivet provision
  [--image] [--document] [--device]` is already the intended home for this.
  **A tool that writes to a fixed or guessed device is worse than one that does
  not write at all**, and `AGENTS.md` §10.4 exists because of that risk.
* **Services belong in the composed document**, chosen by role. A fleet server
  needs `rivet-proxyd`, `rivet-fleetd` and `rivet-mdnsd`; a kiosk needs
  `rivet-launcher` and a `[kiosk]` table; every remotely-administered machine
  needs `[control-plane] listen = "0.0.0.0:7777"`. **The default that produced
  an unreachable board is the bug**, not the operator's omission.
* **The device record.** The first record is a card-time write per `ADR-034`,
  and there is no recovery path for a machine provisioned without it — see
  Findings §2.6, which is the larger and related hole. **These two should
  probably be designed together**, because a fleet server that can be enrolled
  after the fact makes the card-time write an optimisation rather than a
  bootstrap.
* **`ca server` composes names from the recorded suffixes** by default.
* **A browser-enrollment step that can be run alone**, so a lost or keyless
  operator certificate does not require re-minting the authority. Findings §3.2.

## 5. How to verify anything here

`tools/provision/rehearse-menu.sh` drives the menu under `script(1)` — an agent
can walk the whole thing without a human. **It writes into the operator's real
Windows certificate store, which is finding §3.4 and must be fixed before it is
used again**, or it will leave more junk certificates in `CurrentUser\My`.

**Do not test against `rivet-01` at `192.168.50.10`.** It is the live production
fleet server and it is serving a television.

## 6. WHAT THURSDAY MUST DELIVER — the maintainer's requirement, 2026-09-07

**This is not a wish list. It is the acceptance test for the next fleet, and
that fleet is the final one.** Stated by the maintainer after a bring-up he
described in the harshest terms as a failure from start to finish
(`.rulings/` R-302), in his words as far as possible.

### 6.1 `rivet-reset` — a clean state, on purpose

A verb that **wipes the old CA, wipes any old machine documents, and wipes any
old register**, putting the workstation in a known-empty state. Today's fleet
inherited a device record from August that nobody asked for, because the
leftovers were lying around and an agent swept them up.

**It must clean the browser too.** The old authority's root and every leaf it
issued come out of the Windows certificate store as part of the reset — not as
a separate manual chore, and not left for the operator to find later with
`certmgr`. Today the maintainer was offered **four** certificates and none of
them worked.

### 6.2 A SCRIPT IN THE THEATRICAL SENSE — lines a human follows

**Clarified by the maintainer, 2026-09-07:** to be clear, the deliverable is not
a shell script but a list of instructions for a human to follow.
*(`.rulings/` R-302.)*

**This is not a `.sh` file and must not be built as one.** It is an ordered list
of steps the maintainer reads and performs — what to type, what to click, what
he should see back — with the **tool** doing the real work at each step. A shell
script that runs the whole thing unattended is the wrong deliverable twice over:
it hides which step failed, and it is the agent doing it for him again, which is
the failure this whole document exists to end.

**The test of each line:** he can perform it without an agent present, and he
can tell from what comes back whether it worked.

Every step below is his sentence:

1. **Reset the CA**, including cleaning the old CA out of the browser (§6.1).
2. **Mint the new CA.**
3. **Tell him to install `root.crt`** — that one is his, deliberately, because
   trusting a root is a decision a person should take.
4. **Install his new browser certificate FOR him** — he clicks OK on the
   Windows prompt and nothing more. Today this did not happen at all and the
   certificate landed with no private key, which is invisible until a browser
   silently declines to offer it.
5. **Provision a new fleet server, correctly** — which means the composed
   document carries what a fleet server actually needs, including
   `[control-plane] listen` and its services. A default that produces an
   unreachable board is the defect.
6. **ACTUALLY SAVE THE CONFIG.** `w` prints to stdout and exits (§3).
7. **ACTUALLY CHECK IT WITH THE PARSER** — not "it looked right".
8. **Create the image.**
9. **Inject the fleet register**, so the machine boots knowing its own fleet.
   Today this took two directories, three files and five `set-owner` calls over
   the control plane, by hand, after the fact.
10. **Ask which removable disk to write to.** Enumerate them and ask. Never
    guess, never take a fixed path.
11. **Write the image.**
12. **Verify the image.**

### 6.3 The boot test

The maintainer's expectation: when the device boots up, `fleetd` comes up running
just like `mcpd` does. *(`.rulings/` R-303.)*

**That is the pass condition.** Not installed, not idle, not failed pending a
credential somebody has to hand-install afterwards — **running**, on the first
boot, from the card, with its own register already in it and its own
credentials already in place.

`rivet-fleetd` reads `server.crt`, `server.key` and `ca.crt` from its data
directory (§2.1 of the findings). Whatever writes the card must put them there.

### 6.4 What this rules out

* **No hand-editing of TOML** at any point in the flow, by him or by an agent.
* **No `apply install-secret` after boot** to make the fleet server work.
* **No agent required.** If a step can only be done by something that reads Rust
  source, that step has failed.
* **No shell script standing in for the tool.** If a step's answer is *"run this
  script that does nine things"*, the nine things belong in the tool and the
  script is hiding that they are not there yet.

### 6.5 Two landmines already found, which Thursday walks straight into

Both were verified on 2026-09-07 and both are in this flow's path.

**`rivet-fleet add identity` ignores `--confirm`.** Its help says the flag is
what makes it act, and that without it *"the plan is printed and nothing
changes."* `add_identity` never reads `request.confirmed`
(`cmd/rivet-fleet/src/main.rs:1893-1941`): the dry run mints the seed and writes
`device.seed`, `device.crt` and `peers`, and the real run then fails because
`cargo rivet identity new` correctly refuses to overwrite a key that may exist
only once. **A person following a script and checking their work first destroys
the artifact.**

**Running `rivet-provision ca` always writes into the real Windows certificate
store.** Proven by removing `certutil` from `PATH` — Windows resolves it from
the system directory anyway. So there is no dry run, no rehearsal, and no way to
practise Thursday's steps without contaminating the store that Thursday is
supposed to start clean. `rivet-reset` (§6.1) is what makes this survivable, and
until it exists every rehearsal makes the problem worse.

---

## 7. BUILT: what the card must carry, and the three first boots

**Ruled by the maintainer 2026-09-07. This is Wave 3 and it is the item that
makes `SPEC-05` §1.1 true** — the expectation that, when the device boots up,
`fleetd` comes up running just like `mcpd` does (`.rulings/` R-303).

**Built the same day. `ADR-049` is the decision** and supersedes this section as
a brief: §7.3's three rulings are in it verbatim, §7.4's ownership hop is
`cmd/rivet-sysd`'s `deliver_staged`, and §7.6's *"what an implementer still
chooses"* was chosen — one bundle per application under `/state/staging`, for a
reason §7.6 could not have known: the supervisor's syscall port has no recursive
directory walker in it and its own documentation asks that it not grow one.

**Two things this section assumed that turned out not to hold**, both recorded
in `ADR-049`: a member and a stand-alone machine stage *nothing*, because the
roles enable no application that reads per-machine credentials and the fleet's
existing register cannot be read off a running server by any verb (§2.6); and
the delivery is usually **not** on first boot, because no application is on a
fresh card at all.

### 7.1 The three first-boot states, as he put them

* **Stand Alone** — on first boot the machine comes up knowing everything it
  needs to get itself up, with its certificates, and able to communicate with
  `cargo-rivet` on the PC, which is its fleet server.
* **Fleet Server** — the machine comes up, provisions the new fleet register,
  enrolls itself, and is ready to talk to `cargo-rivet` with credentials.
* **Member Server** — the machine boots with the fleet's existing register and
  its certificates.

*(`.rulings/` R-303.)*

### 7.2 They are ONE mechanism, and it already exists

**Do not design a new one.** `cmd/rivet-image`'s `--identity DIR` already states
the pattern: *"They go on the BOOT partition; PID 1 copies them to
`/state/secrets` on first boot and DELETES the seed from the card (`ADR-019`
§3.1)."* A card-time staging area, a courier that runs before any service, and a
discipline for destroying the secret half — already trusted with `device.seed`.

The three states differ only in **what is staged**:

| | register staged | additionally |
| :--- | :--- | :--- |
| Fleet Server | one record — its own | fleetd's `server.crt`/`server.key`/`ca.crt`, its `operator-device.seed.hex` and `.crt.hex`, and its public key in the peers file |
| Member | the fleet's existing records | its own leaf and the fleet root |
| Stand Alone | **none** — see §7.3 | its own leaf and the fleet root |

### 7.3 The maintainer's three rulings

1. **Stand Alone carries NO register.** A machine that does not need one does
   not carry one (`.rulings/` R-303). The PC holds it. He had assumed symmetry with the other two; the symmetry would have
   created a second authority for something the PC owns, which is what
   `ADR-034` §2.2 dropped Local mode to avoid.
2. **Secrets are DELETED from the card after first boot** — the same discipline
   `device.seed` already gets, applied to `server.key` and
   `operator-device.seed.hex`.
3. **`peers` is not a separate problem.** `--identity DIR` already takes
   *"device.seed, device.crt and a peers file beside them. ALL THREE OR NONE"*
   and PID 1 already copies all three. A fleet server needs nothing new here:
   `rivet-provision` puts fleetd's public key into the peers file it already
   writes. **This was raised as an open question and was not one** — the agent
   that raised it had quoted the answer two paragraphs earlier.

### 7.4 The one thing that IS new: ownership

**An application's data directory is `0700` owned by that application's uid**
(`ADR-015` §5.1.1), and the uid is allocated by the supervisor at discovery. It
is not knowable at image-build time and not knowable by PID 1 either. So the
courier needs one hop more than identity does:

    boot partition → PID 1 → staging under /state → the SUPERVISOR materialises
                                                    into each application's data
                                                    directory, with the uid it
                                                    has just allocated

**This is not theoretical.** On 2026-09-07 the register and credentials were
placed by hand as root, and `rivet-fleetd` could not read a single file of it.
**It logged nothing**, because `rivetos_register::store::load` treats a missing
directory as an empty register by design — correct for a first run and silent
for this. Five `apply set-owner` calls fixed it.

### 7.5 A fleet server cannot self-enroll, so "provision the register" is wrong

`cmd/rivet-fleetd/src/fleet.rs` refuses by design: *"It does not invent entries.
A fleet server that met an empty directory and minted a record for itself would
be manufacturing the one thing `ADR-034` §3.5 says it cannot have: a device
record is written by the tool that GENERATED the device's keypair, and this
program generated nobody's."*

So the Fleet Server case is **not** *come up and create a register*. It is *come
up carrying a one-record register that `rivet-provision` wrote when it minted
your identity*. That is what makes all three states the same shape.

### 7.6 What is left to decide

**Little.** §7.3 settles the questions that were open. What an implementer still
chooses is the **staging layout** — where under `/state` the material waits
between PID 1 and the supervisor, and how the supervisor knows which staged
directory belongs to which application. That is an implementation shape rather
than a ruling, and it belongs in the ADR that records §7.3 rather than in a
document asking him anything further.

---

## 8. Constraints on whoever takes this

* Never call `spawn_task`; put follow-up work in the final report as prose.
* Never use a heredoc. Use Write/Edit and `uv run`.
* One command per shell call. Never chain with `&&`; never `cd`.
* Never touch an SSH key.
* Python is always `uv`.
* Do not modify `~/.rivet/ca` — it is the live fleet authority.
* Read every tool's `--help` before writing that it cannot do something.
  Findings §3.5 is what happens otherwise.
