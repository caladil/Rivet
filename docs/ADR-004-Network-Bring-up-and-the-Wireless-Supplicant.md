# ADR-004: Network Bring-up Order and the Wireless Supplicant

* **Status:** Accepted. **§6.1 added 2026-08-23** — the first-boot provisioning
  channel carries the `[network]` table too; `ADR-007` §7.3 is the schema.
  **§6.2 added 2026-09-07** — what a document must CONTAIN for the role it was
  composed for, after a fleet server booted unreachable; `SPEC-05` §5 is the
  acceptance test.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet's first meaningful milestone is a host that can be reached over the
network — today that means its control plane answering a peer that has its key.
The eventual and intended link is wireless; the Pi 4's wired interface is a
bring-up scaffold, and the deployed configuration has no expectation of a
permanent cable.

The question is what order to bring these up in, and how much of the wireless
stack the project should own. Both answers turned out to be the opposite of the
first instinct.

---

## 2. Decision: bring-up order is init, then wired, then wireless

**Stage order is `init` then `eth0` then `wlan0`.** Wireless is not attempted
until a host has been reached over the wired interface.

The reasoning is about isolating variables, not about convenience.

### 2.1 Almost nothing in the path is link-specific

Everything that must work before a remote operator gets an answer sits at layer 3
or above and is indifferent to what carries the frames:

* netlink: interface up, address assignment, default route
* a DHCP client
* DNS resolution and `resolv.conf`
* the control-plane listener: a device identity, the peers it accepts, and a
  socket bound somewhere a workstation can reach
* mDNS responder, so the host is reachable by name

Ethernet is layer 2. DHCP and everything above it neither knows nor cares what
is underneath. Bringing the wired interface up first therefore exercises the
entire userland network stack against a link that **cannot fail to associate**.

### 2.2 The cost of not doing it

Attempting wireless first means the first failure presents as *nothing answers*,
with at least five candidate causes: the firmware blob, the driver, the
supplicant, DHCP, or the control-plane service's own bring-up. One symptom, five
unknowns, no way to bisect.

With the wired stage complete, wireless introduces exactly one new variable:
association.

### 2.3 Cost and tripwire

The Pi 4's `bcmgenet` driver is in-kernel, device-tree described, and needs no
firmware blob or NVRAM file. There is nothing to configure on the host side.

**Tripwire:** if `eth0` does not appear in `ip link` on the first boot with a
stock kernel, the wired stage is abandoned and work proceeds directly to
wireless. That is a fifteen-minute diagnosis with a binary answer, not an
open-ended hunt. It bounds the scaffold's cost.

**It did not fire — checked 2026-08-22 on a Pi 4 Model B Rev 1.1. See §8.2.**

### 2.4 The wired interface is not removed afterwards

Unlike the rejected proposal in section 7, `eth0` costs nothing to keep: no
code, no configuration, no additional component. It remains as the recovery
path for the first time wireless credentials are entered wrongly.

---

## 3. Decision: `wpa_supplicant` is a permanent Tier C dependency

**Rivet does not implement WPA association. `wpa_supplicant` is adopted as a
Tier C dependency (`ADR-001` section 4) with no replacement intended.**

### 3.1 Why not implement it

WPA2-PSK requires the four-way EAPOL handshake in userspace: PMK derivation,
nonce exchange, PTK and GTK installation, and pushing keys back into the kernel.
`wpa_supplicant` is thousands of lines of C for reasons that are all good ones.

The decisive argument is not effort, it is **currency**. Depending on
`wpa_supplicant` is what keeps Rivet on WPA3-SAE, 802.11r/k/v roaming and
enterprise EAP, and whatever follows them, at no ongoing cost. Implementing our
own is what would strand the project on WPA2-PSK permanently — we would
implement PSK, declare victory, and never revisit it. **The C dependency is the
future-proofing, not the compromise.**

This is also the load-bearing link in the deployed configuration, which raises
rather than lowers the argument against hand-rolling it.

### 3.2 Why not `iwd`

| | `wpa_supplicant` | `iwd` |
| :-- | :-- | :-- |
| Control path | Unix socket, plain text protocol | D-Bus |
| DHCP | external | integrated |
| Codebase | older, larger | newer, cleaner |
| Additional daemons required | none | a message bus |

`iwd` is the nicer codebase and its integrated DHCP client would remove a
component. It is rejected because its control interface is D-Bus, and adding a
message bus daemon to the image in order to configure wireless is a worse trade
than accepting a larger supplicant. The socket protocol is also far easier to
drive from Rust without pulling in a D-Bus stack.

---

## 4. Decision: the boundary is a socket, not an FFI linkage

`wpa_supplicant` is a daemon, not a library. Rivet therefore talks to it over
its control socket rather than linking it.

```
ATTACH
ADD_NETWORK               -> 0
SET_NETWORK 0 ssid "..."
SET_NETWORK 0 psk <64 hex chars>
ENABLE_NETWORK 0
CTRL-EVENT-CONNECTED - Connection to ... completed
```

This is the cleanest boundary available and it satisfies `ADR-001` section 5
exactly: no `unsafe`, no ABI coupling, no `bindgen`, no C in the Rust build
graph, and independent lifecycle and restart.

The supervisor owns the supplicant process — one service per wireless
interface, §8 — and `rivet-netd` drives the socket and translates its events
into Rivet's own types. The protocol is small enough that Rivet owns the client
outright rather than taking a crate dependency (`rivet_adapter_net::supplicant`).

The supplicant is built **without D-Bus support** and with only the EAP methods
actually required.

---

## 5. Decision: credentials are Rivet's, and the passphrase is not stored

**`wpa_supplicant.conf` does not exist on a Rivet system.** Networks are
configured in Rivet's own configuration, in Rivet's own format, and the
supplicant is driven at runtime. Its configuration file is an implementation
detail that has been removed rather than generated.

**The plaintext passphrase is never written to persistent storage.** Rivet
performs the PBKDF2 derivation itself and stores only the 32-byte PMK, which
`wpa_supplicant` accepts directly in its `psk=<hex>` form.

Storing plaintext pre-shared keys in `/etc` is near-universal practice among
distributions and it should not be. This costs one well-specified key
derivation to avoid.

**Generalised 2026-08-22 by `ADR-020`.** The PMK is now one instance of a
general mechanism: it is a `Secret` in the schema, so it cannot be rendered into
`plan`, `export` or a log, and its material lives in the store at
`/state/secrets` rather than in the configuration document.

**Corrected 2026-08-30, and the correction matters here more than anywhere.**
That paragraph said *the encrypted store*, and there is no encryption:
`ADR-020` §11.2.3 removed the envelope and the `KeyProvider` after a real
Raspberry Pi OS box was measured and found keeping its wireless passphrase in a
`0600` root-owned file in plain text. What `/state/secrets` gives a PMK is a
root-owned file holding **hex, which is an encoding and never protection**.

**This does not weaken §5's own rule, it is the reason for it.** The passphrase
is still never persisted, and that was always the half worth having: possessing
the PMK joins this network, while the passphrase is the thing a human has
probably reused somewhere else. Obfuscation stops the casual reader of a card;
PBKDF2 is what stops the loss reaching beyond this LAN. Only the second of
those was ever load-bearing.

---

## 6. Decision: first-boot provisioning comes from the boot partition

A headless host with no wired link cannot be given credentials after the fact,
and the development host is a Windows machine with no native ext4 access.

**Rivet reads a provisioning file, `rivet.toml`, from the FAT boot
partition** (`ADR-002` section 4). It is editable from any host operating system
with a text editor and no additional tooling.

The provisioning sequence, which composes with section 5:

1. PID 1 mounts the boot partition and reads `rivet.toml`.
2. The passphrase is read, the PMK derived, and the result written to the
   persistent configuration.
3. **The credential is then removed from the provisioning file**, so plaintext
   exists only between the operator typing it and the first boot.

This is the same mechanism as the binary transfer channel in `ADR-002`
section 4.1 — one mechanism, two jobs, no new component.

### 6.1 Added 2026-08-23: the network case, which is the same mechanism again

The sequence above was written for wireless credentials. It generalises without
change, and the maintainer ruled on 2026-08-23 that it must —
option 2, *a `[network]` table plus the provisioning file*, with the
recommendation's own words being *"one mechanism, two jobs"*, which is section 6
quoted back at itself.

**The problem it solves is section 6's problem, one step earlier.** A headless
host with no wired link cannot be given credentials after the fact; a headless
host with no *address* cannot be given anything after the fact, including the
credentials. Wireless was the first thing that needed a first-boot channel and
it is not the only one.

So `rivet.toml` on the boot partition carries a `[network]` table — one
sub-table per interface, `method = "dhcp"` or `method = "static"` with an
`address`. `ADR-007` §7.3 is the schema and §7.4 is why `method` is a key rather
than an inference. Nothing new is built: the file, the mount, the parser and the
refusal path all existed for the wireless case.

**Where it differs from the credential, and it differs in one way that matters.**
Section 6 step 3 removes the credential from the file after the first boot,
because a passphrase in plaintext on a FAT partition is a liability that should
exist only between the operator typing it and the machine reading it. **An
address is not a secret and is not removed.** It is ordinary desired state, and
a machine whose provisioning file still describes the network it is on is a
machine that can be re-imaged onto the same network without being told twice.

**Step 2 — *"the result written to the persistent configuration"* — is built.**
`rivet-store` exists, `ADR-017` §3.1.8 grew PID 1's job list to include the
write, and on a first boot the provisioning document becomes
`/state/config/machine.toml`.

**The console still reports by count and never by value**, and that half of it
was never about `rivet-store`: the file holds a passphrase, so a console — which
is a log — gets an interface count (`ADR-020` §4).

**What is still not built, and the list is short.**

1. **Step 3 is outstanding, and it has nothing to remove yet.** `ADR-007` §7.3
   keeps wireless credentials out of the configuration document entirely, so the
   schema declares no key a passphrase could be written under and a
   `rivet.toml` carrying one is refused at parse time as unknown. Step 3
   becomes implementable when `ADR-020`'s encrypted store and a `Secret`-typed
   schema key exist. `ADR-007` §7.6.3 carries the reasoning.
2. **The stored document is read and applied — closed 2026-08-23**, and it is
   listed here because it was open when this list was written.
   `ADR-017` §4.3. `rivet-sysd`
   compiles in a service definition for `cmd/rivet-netd`, reads
   `/state/config/machine.toml` itself before it converges anything, and starts
   `netd` with that path. A first boot now stores what it was told **and applies
   it**: observed under QEMU with `ADR-024`'s own kernel, `eth0` brought up,
   addressed from the document, routed, and read back — *configured, carrier
   present*.

   **`bcmgenet` is untouched by that**, and §2.3's tripwire is about the board.
   The emulated link was `virtio_net` (`ADR-026` §4).
3. **Seeding never overwrites.** An address is not removed from the provisioning
   file (above), so the file is still there on every later boot — and applying it
   each time would make the boot partition authoritative for configuration
   (`ADR-011` §3) and discard whatever the operator had since set (`ADR-007`
   §3.6.5). The store says so on the console and does nothing.
4. **A stored document that cannot be read is refused rather than replaced.**
   That is `ADR-018` §8's recovery path and `ADR-018` §9 item 3 keeps it open;
   the store names the open item in its refusal rather than guessing at it.

**None of this has run on hardware, and QEMU cannot exercise it** — the emulator
has no boot partition, so it offers no provisioning document at all.

### 6.2 Added 2026-09-07: the ROLE composes the document, and a schema default is not one

Section 6 and §6.1 settle *what channel* a first boot is configured over. They
say nothing about *what a document should contain*, and by 2026-09-07 that
omission had a measured cost: a fleet server was provisioned, booted, and
answered nothing. **The only recovery was rebuilding the card and rewriting it**
— a headless machine cannot be told anything after the fact, which is §6's own
first sentence arriving one layer up.

**The decision: `rivet-provision` composes a document for the ROLE the operator
chose, and the schema's defaults are not what a provisioned machine gets.**
`SPEC-05` §5 is the list; this is why the list exists and why each entry is
where it is.

#### 6.2.1 The control-plane listener is decided by who administers the machine

**`[control-plane] listen` is written into every composed document**, and its
value is `0.0.0.0:7777` for a fleet `server` or `member` and
`rivet_config::control_plane::DEFAULT_LISTEN` — loopback — for a `stand-alone`
machine.

**A default that bricks a headless machine is a defect, not an operator's
omission.** There is no local administration surface on a Rivet: `ADR-006` §5
makes the control plane the transport and the CLI a remote API, and `ADR-032` is
the whole of *asking a running machine*. A board listening on loopback is
therefore not a private machine — it is a machine nobody, including its owner,
can reach. That is what happened.

**And the schema default STAYS loopback**, which is the half that is easy to get
wrong by fixing the visible thing. `rivet_config`'s `DEFAULT_LISTEN` is what
applies when a document says nothing, and `ADR-018` §8 makes that the state a
machine falls into when its stored document cannot be read. A machine that has
lost its configuration must not begin answering the network because a file went
missing; the safe direction for *absence* is the opposite of the safe direction
for *a role somebody chose*. Changing the constant would have made every
recovery path louder in exactly the wrong way, and it would have done it
silently, since nothing on a booting board announces which default it fell back
to.

**Loopback is also kept for `stand-alone`, for a second reason that is not
squeamishness.** `cmd/rivet-mcpd` refuses a non-loopback address when it was
given no TLS identity, by name, before it binds (`ADR-035` §7.2). A stand-alone
machine's card may carry no identity — nothing in the stand-alone path mints
one — so composing `0.0.0.0:7777` there would produce a machine with **no
control plane at all** rather than one reachable only from itself. That is
strictly worse. `SPEC-05` §8 already puts stand-alone out of scope for the first
fleet, and this is the shape that costs nothing when it is brought back in.

**The coupling is stated on screen rather than assumed.** A document that binds
every interface is only correct on a card that carries an identity
(`rivet-image --identity`, or `RIVET_IDENTITY` for
`tools/container/build-card.sh`), so `rivet-provision` says so at the moment it
composes one. An operator who reads nothing else reads the four lines under the
document they just asked for.

**The table is written out even when its value is the default.** An absent
`[control-plane]` cannot be edited: the repair for a machine listening in the
wrong place is `apply edit`, which takes a path and `--expect`/`--replace` or
`--append`, and appending a table to a document that ends in one puts the new
keys inside the last table instead of at the top level. A key that exists can be
changed. A table that is missing has to be composed by hand, on the machine
whose control plane is the thing being repaired.

#### 6.2.2 The timezone is ASKED, and it cannot be a default

**`rivet-provision` refuses to compose a document until the operator has named a
zone**, and the composed document carries the top-level `timezone` key. The
maintainer ruled on 2026-09-07 that the zone is set for the board in hand and
that every later provisioning asks for it as a setting of its own.
*(`.rulings/` R-261.)*

**It is card-time work because the projection is.** `/etc/localtime` is copied
out of the image's zone database into a tmpfs **before any service starts**, and
the tmpfs is then remounted read-only (`ADR-007` §3.5.1, §3.6.1). Setting the
key afterwards changes nothing until the machine restarts — which `rivet-01`
needed on 2026-09-07, with Aura already on a television showing the wrong time.

**Silence is the expensive failure, and that is why there is no default.** musl,
Go and chrono all read that exact path when `TZ` is unset, and every one of them
keeps UTC **silently** when it is absent (`ADR-021` §9). Nothing fails. Every
timestamp is simply wrong, including the ones somebody would use to work out
why. A field carrying a plausible default would be answered by silence, and
silence is the defect — so the field begins refused, and `UTC` is a *typed*
answer rather than the absence of one.

**It is not guessed from anything to hand.** A zip code is collected for other
reasons and the workstation has a zone of its own; neither is the machine's.
Eleven zones are offered by number and any of the database's six hundred may be
typed.

**And the name is checked against zones that exist, which no other check in this
workspace makes.** `rivet_core::timezone::Timezone` enforces the grammar and
says plainly that it cannot know whether a zone exists — a document is written
for machines that have not been built yet. That leaves `America/Chigago`: a
well-formed name for a file no image carries, which parses, projects, maps
nothing, and produces a UTC clock under a document naming a zone. So
`cmd/rivet-provision/src/zones.rs` holds the name against a real
`zoneinfo` directory when one is readable — which is the case in the container
where cards are built — and against a generated list of the card database's own
599 names when one is not, which is the case on a Windows workstation. It says
which of the two answered, and a refusal names the zone the operator meant.

**`timezone` is written above the first table header**, because a bare key in
TOML belongs to whichever table precedes it. This is also why it cannot be added
to a finished document afterwards, and why `examples/aura/aura-controller`'s
General page — which tells a person to run `rivet apply edit timezone <zone>
confirm`, a shape `apply edit`'s grammar has no room for, since it takes a path
and `--expect`/`--replace` or `--append` — is a defect in that program.

#### 6.2.3 A fleet server turns on three things, and none of them is on by default

`[services.rivet-fleetd]`, `[services.rivet-proxyd]` and
`[services.rivet-mdnsd]`, all `enabled = true`. The third is the one that looks
optional and is not: a fleet server's certificate carries `<hostname>.<suffix>`,
a browser validates the name it asked for, and `rivet-mdnsd` is what publishes
that name. Without it the failure appears as a name a browser cannot find rather
than as a service that is off. A fleet `member` gets `rivet-mdnsd` for the same
reason — it is reached by name, by the server that manages it and by a person.

#### 6.2.4 A kiosk names the application that takes the display, and enables it

`[kiosk]` is written for a machine whose mode is `kiosk`, empty if the operator
has not decided. When `auto-run` names an application, **the same application is
enabled in `[services.<name>]`** — the supervisor starts nothing the document
has not enabled (`ADR-015` §3.2.1), so an `auto-run` naming an installed but
disabled application is the same black screen as one naming an absent
application, arrived at by a different route.

**What this tool cannot do, stated rather than guessed at:** nothing here can
know whether the named application will be installed. At card time none of them
are, and the manifest that would say so is written by an install that happens
afterwards. Naming an absent
application takes the display and draws nothing on it — no message on the
screen, and nothing in the journal that says so plainly — which happened on
2026-09-07 and cost a card rebuild. So the tool **asks**, offers
`rivet-launcher` as the one application it can name without guessing at a
payload, accepts blank as a real answer, and prints what the risk is. Closing
this properly needs the card build and the provisioning document to know about
each other, which is `SPEC-05` §4.8 and §4.9 and is not this section's.

#### 6.2.5 What was rejected

* **Changing `rivet_config::control_plane::DEFAULT_LISTEN` to `0.0.0.0:7777`.**
  It fixes the visible symptom and breaks the invisible case: the default is the
  lost-document fallback, and §6.2.1 is the argument.
* **A per-mode listener** — `headless` binds, `kiosk` does not. Mode says
  whether a screen is attached, not who administers the machine, and a kiosk is
  administered remotely exactly as a headless board is. The `Fleet` field is the
  one that answers the question being asked.
* **Guessing the timezone from the workstation.** Rejected by the maintainer's
  ruling; §6.2.2.
* **Composing `[kiosk] auto-run = "rivet-launcher"` unasked**, on the grounds
  that the launcher is usually right. It is a guess about what will be
  installed, and the failure it produces is a black screen with no message.

---

## 7. Rejected: USB gadget networking

An earlier proposal used the target's USB OTG port in device mode
(`dtoverlay=dwc2` with `libcomposite`) to present a composite CDC-ACM serial
console and USB Ethernet interface to the development host, giving a console and
a network path over a single cable.

**Rejected.** The reasoning is kept because it is the sharper form of the
argument in section 2, and because the failure mode is one this project will
meet again.

* **It shares almost nothing with the wireless path.** Static addressing on
  both ends because no DHCP server exists on the link, host-side RNDIS or NCM
  driver behaviour, and a `libcomposite` stack — none of that knowledge or code
  carries forward. Wired Ethernet, by contrast, shares the entire layer-3 stack
  (section 2.1). One scaffold is reusable and one is not.
* **It optimises an iteration loop that does not exist yet.** During Stage 0 the
  component under test *is* PID 1, which cannot be hot-reloaded, so every
  iteration is a reboot regardless and a network push saves nothing. By the time
  iteration speed matters, the work in hand is `rivet-netd` — meaning wireless
  is the very next thing to come up anyway. The window in which the gadget helps
  is narrow, and opening it costs a day of host-side driver work.
* **It was inherited reasoning, not current reasoning.** The proposal was
  correct for the Pi Zero W, which has no wired NIC and no usable video output,
  and it was carried forward to the Pi 4 without being re-derived against a
  board that has both. **A decision that was right for the previous hardware is
  not evidence about the current hardware.**

Replaced by: micro-HDMI console for Stage 0, serial UART for panics, and the
boot partition for file transfer (`ADR-002` section 4.1).

---

## 8. Consequences and recorded risks

* **The wireless firmware requirement is three files, not two.** The Pi 4's
  CYW43455 needs `brcmfmac43455-sdio.bin`, the Raspberry Pi specific NVRAM
  `.txt`, **and `brcmfmac43455-sdio.clm_blob`**. A missing `clm_blob` is a quiet
  failure: the driver loads and the interface appears, but regulatory data is
  absent and association misbehaves. Recent kernels also resolve the NVRAM file
  through a board-specific path derived from the device-tree compatible string,
  so a correct file under the wrong name reads as absent. This is the fiddliest
  step in Stage 2 and should be budgeted accordingly. These failures appear only
  in kernel messages, which is why `ADR-022` §4.1 has the journal ingest
  `/dev/kmsg` — the fiddliest diagnosis in the project is then queryable through
  the same tool as everything else. **READ FROM THE BOARD 2026-08-22; the exact
  names are in §8.1.**
* **A console is required before either network stage.** Stage 0 has no network
  by definition. Micro-HDMI plus a USB keyboard is sufficient and requires no
  purchase. A serial UART adapter on GPIO 14/15 at 115200 is still wanted, for a
  reason worth stating precisely: with HDMI a panic is *photographed*, with
  serial it is *pasted*. That difference matters when the panic is being
  diagnosed in a chat transcript at midnight.
* **DHCP is BusyBox's `udhcpc`, a mechanism Rivet drives**, chosen after the
  Rust clients were surveyed: the one maintained crate is a codec that brings an
  internationalisation stack with it and leaves the state machine and the
  T1/T2 timers to be written anyway, and the rest are abandoned or
  server-shaped. `rivet_adapter_net::lease` records that survey. `udhcpc` was
  chosen over `dhcpcd` because **it configures nothing**: it speaks the
  protocol and execs a program with the lease in its environment, so every
  decision about the answer stays Rivet's.

* **The DHCP client and the supplicant are supervised services, one per
  interface that needs one** (`.rulings/` R-253). The supervisor derives
  `rivet-dhcp-<interface>` for each interface the document addresses by DHCP,
  and `rivet-wpa-<interface>` for each it gives an `ssid`, from the document's
  `[network]` table at boot; it keeps each running and restarts it whatever its
  exit. `ADR-017` §4 records the definitions. Three programs share the work,
  and exactly one of them writes netlink:

  | Program | Does | Lifetime |
  | :-- | :-- | :-- |
  | `udhcpc` | takes a lease, renews it at T1, rebinds at T2; never quits on its own (no `-q`, no `-n`) | a supervised service |
  | `cmd/rivet-dhcp-hook` | the program `udhcpc` execs per event: writes the lease to `/run/rivet/dhcp/<interface>.lease` or removes it, and sends the supervisor `SIGHUP` either way | one event |
  | `cmd/rivet-netd` | raises each link, applies static addresses, applies each recorded lease's address and route and withdraws every other IPv4 address and default route from that interface, tells each supplicant its network | runs once, and is run again |

  **Where "apply the lease" lives, and why.** In `rivet-netd`, the program that
  already writes every static address, so one program owns *what address an
  interface has* (`ADR-011`). The hook's contract is unchanged — it records and
  nudges, and performs no netlink. The nudge is a reload, and the reload
  compares each DHCP interface's lease file with what was last recorded and
  runs `rivet-netd` again when one is new, different or gone. That is how a
  first lease, a renewal that changes the address, the router or the mask,
  and a lease withdrawn by `nak` or `leasefail` reach the interface; a renewal
  that rewrites the same lease runs nothing. The two
  alternatives were weighed and rejected: the hook writing netlink itself would
  make two programs able to set one interface's address, and a long-lived
  `rivet-netd` holding `CAP_NET_ADMIN` for the machine's whole life is what
  `ADR-017` §3's reduction of its privileges exists to avoid.

  **A supplicant's network lives only in its memory**, because it is started
  with no configuration file and the network is told to it over the control
  socket (§4, §5). So the supervisor also runs `rivet-netd` again
  whenever it starts a supplicant after `rivet-netd` has already run.

  **Why services and not `rivet-netd`'s children.** `rivet-netd` runs once and
  exits, and the supervisor then sweeps its cgroup for anything left behind
  (`ADR-017` §3). A client or supplicant `rivet-netd` started sat in that
  cgroup and was signalled with it, so a lease was never renewed and an
  association would end with the program that made it. As services they have
  cgroups of their own, output captured for their whole life, and a restart
  when they die.

  **A DHCP interface carries exactly its lease's address.** Each time
  `rivet-netd` runs it lists the interface's IPv4 addresses and withdraws every
  one that is not the lease's — matched by address and prefix length — and
  then assigns the lease's. So a withdrawn lease leaves the interface with no
  address, and a renewal that moved the address leaves only the new one. The
  stale address goes first, because deleting an interface's primary address
  takes the secondaries in its subnet with it unless the kernel is told to
  promote them. A withdrawal the kernel refuses exits `rivet-netd` non-zero, so
  the supervisor runs it again.

  **A DHCP interface's default route goes via exactly its lease's router.**
  Before it applies the lease, `rivet-netd` lists the main table's IPv4
  default routes and deletes every one that leaves by that interface and does
  not go via the lease's router; then it adds the lease's. So a renewal that
  changes only the router leaves a single default route, via the new one, and
  a lease with no router, or no lease, leaves the interface none. A default
  route leaving by another interface — the one a static interface's `gateway`
  installs — is never touched, whichever router it names. A deletion the
  kernel refuses exits `rivet-netd` non-zero, so the supervisor runs it again.

  **Delete and then add, rather than one replacing request.** The kernel
  matches an `NLM_F_REPLACE` on the table, the destination, the type of
  service and the metric, never on the interface, so a replace issued for the
  DHCP interface would overwrite a static interface's default route of the
  same metric. Deleting first is also what makes the add land: while any
  default route of that metric stands, the exclusive add is refused as already
  present. The default route is absent between the two requests; a session
  from the interface's own subnet is delivered on-link and does not use it.

  **Boot.** The client starts in the same convergence pass as `rivet-netd`,
  which raises the link it broadcasts on. `rivet-netd` does not wait for a
  carrier: the client keeps sending DISCOVERs on its own schedule, so a link
  whose carrier arrives seconds after it is raised is answered at the next one.

---

## 8.1 The firmware filenames, read from the board — 2026-08-22

Read from a Raspberry Pi 4 Model B Rev 1.1 running stock Raspberry Pi OS
(kernel `6.18.34+rpt-rpi-v8`), per §8's requirement that these come from
`dmesg` and the filesystem rather than from documentation.

**The chip.** `brcmfmac: F1 signature read @0x18000000=0x15264345`, and
`brcmf_fw_alloc_request: using brcm/brcmfmac43455-sdio for chip BCM4345/6`.
The running firmware reports `BCM4345/6 wl0: Aug 29 2023 ... version 7.45.265`.

**The naming rule, confirmed.** The driver takes the base name
`brcm/brcmfmac43455-sdio` and appends the **first device-tree compatible
string** — `/proc/device-tree/compatible` reads `raspberrypi,4-model-b` then
`brcm,bcm2711`. So the names the kernel actually looks for on this board are:

| File | Resolves to | Required |
| :-- | :-- | :-- |
| `brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.bin` | `cypress/cyfmac43455-sdio-standard.bin` | **yes** |
| `brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.txt` | `brcm/brcmfmac43455-sdio.txt` (NVRAM) | **yes** |
| `brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.clm_blob` | `cypress/cyfmac43455-sdio.clm_blob` | **yes** |
| `brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.txcap_blob` | — absent — | **no** |

**§8's "three files, not two" is correct**, and there is a fourth the driver
asks for and does not need: `brcmf_c_process_txcap_blob: no txcap_blob
available (err=-2)`, after which firmware initialisation completes normally.
A fourth file being optional is worth knowing precisely because §8's warning is
that a missing file is a *quiet* failure — this one is quiet and harmless, and
mistaking it for the `clm_blob` case would send a Stage 2 diagnosis the wrong
way.

**Two details that matter for image assembly.** On Raspberry Pi OS every one of
these is a **symlink**, and the payloads live under `cypress/` with different
names — the `.bin` resolving to `cyfmac43455-sdio-standard.bin` implies a
`-standard` variant chosen among alternatives. Rivet's image must ship real
files under the names the driver requests, and must decide deliberately which
Cypress variant it ships rather than inheriting Raspberry Pi OS's choice.

**Unproven:** association itself. `wlan0` exists and is `DOWN`; the firmware
load path is confirmed end to end, but no network was joined, so §8's warning
that a missing `clm_blob` makes "association misbehave" remains untested.

## 8.2 The Stage 1 tripwire did not fire — 2026-08-22

§2.3's binary question, answered on the same board: `ip -br link` reports
`eth0 UP dc:a6:32:xx:xx:xx <BROADCAST,MULTICAST,UP,LOWER_UP>` (the board's own
bytes masked). `eth0` appears
with a stock kernel, `CONFIG_BCMGENET=y` is confirmed in the stock
configuration, and the wired path Stage 1 is built on is available. `wlan0` is
present too, so Stage 2's driver binds on the same stock kernel.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### 2026-10-04 — §8: a renewal that moves the router moves the default route

When a withdrawn or moved lease first took its address off, §8 recorded one
gap left:

~~**What this does not do.** A renewal that changes only the router leaves the
old default route in place: the new one is refused as already present, which
`cmd/rivet-netd` reads as converged.~~

**Why it changed.** The machine kept sending off-subnet traffic to a router
its lease no longer named, which may be gone or may be another host. The
exclusive add that keeps `rivet-netd` from overwriting a route it did not
install is also what refused the new one, so the stale route now goes first,
scoped to the DHCP interface so a static interface's default is never
touched.

### 2026-10-04 — §8: a withdrawn or moved lease takes its address off

When the client and the supplicant first became services, §8 recorded two gaps:

~~A lease that is withdrawn — `udhcpc` removing the file on `nak` or
`leasefail` — is not followed: nothing removes the address from the interface,
and the client is already asking for the next lease. A renewal that changes the
address adds the new one; the old address is not withdrawn.~~

**Why it changed.** Both left an interface answering on an address its lease no
longer named, which a DHCP server may already have given to another machine.
`rivet-netd` is the one program that writes an interface's address, so it now
withdraws what the lease does not name each time it runs, and the supervisor
runs it when a lease goes away as well as when one arrives or changes.

### 2026-10-04 — §4 and §8: the client and the supplicant became services

`.rulings/` R-253. Until this date `rivet-netd` started `udhcpc` and
`wpa_supplicant` itself, waited for a lease or an association, applied it and
exited, leaving both running as orphans. §4 read:

~~`rivet-netd` owns the supplicant process, drives the socket, and translates
its events into Rivet's own types.~~

**Why it changed.** The supervisor sweeps a service's cgroup after it exits,
so that a helper it forked cannot outlive it holding granted devices and
sockets. That sweep found `udhcpc` in `rivet-netd`'s cgroup five seconds into
every DHCP boot and signalled it, so a lease was never renewed; a supplicant
started the same way was in the same cgroup. Exempting the network's
mechanisms from the sweep would have kept them unsupervised — no restart, and
their output discarded once `rivet-netd`'s pipe closed — so they became
services instead, and `rivet-netd` became something the supervisor runs again
when a lease changes.

### 2026-08-30 — §8: the DHCP stopgap decision reversed

The §8 bullet ruled on 2026-08-26 read:

~~**No C DHCP stopgap is taken; the Rust client is the only implementation.**~~

It was reversed four days later, once the Rust clients had been surveyed and
measured rather than assumed: none was a client, and adopting the one
maintained codec would still have meant writing the protocol's state machine.
`udhcpc` was taken, and §8 now states why.

### 2026-09-07 — §6.2 reverses what a composed document listened on

**Nothing in the body was superseded; a behaviour was.** Until this date
`rivet-provision` composed no `[control-plane]` table at all, so every document
it wrote fell to `rivet_config`'s `127.0.0.1:7777` — and a fleet server built
from one booted, came up correctly, and answered nothing. The only recovery was
rebuilding the card, because a headless machine cannot be told anything after
the fact.

**It is recorded here rather than left to `git log` because reversing a default
is a decision**, and without this entry §6.2.1's insistence that the *schema*
default stays loopback reads as an inconsistency rather than as the point: the
two defaults answer different questions, and the code changed on only one of
them. The tool now states the address it chose and why, and the constant is
untouched.

### In §6.1 — step 2 had no implementation

**Changed later on 2026-08-23**, when `rivet-store` was created and PID 1 grew
the write. The superseded paragraph read:

~~**What is not built, said plainly.** Step 2 — *"the result written to the
persistent configuration"* — has no implementation for either case. `ADR-014`
§2's `rivet-store` does not exist; checked 2026-08-23 two ways per
`AGENTS.md` §7, against the workspace member list in the root `Cargo.toml` and
against `git ls-files`. `cmd/rivet-initd` reads the document, validates it,
reports on the console how many interfaces it declared — by count, never by
value, since the same file holds the passphrase — and then drops it.~~

It was struck rather than deleted because `AGENTS.md` §7 is about a claim of
absence outliving the absence, and this one outlived it by hours. The console
half of it is still true and is stated in the body.

### In §6.1 item 2 — nothing read the stored document

**Changed later on 2026-08-23** — `ADR-017` §4.3, when `rivet-sysd` grew a
compiled-in service definition for `cmd/rivet-netd`. The superseded item read:

~~**Nothing reads the stored document.** `cmd/rivet-netd` is told a
`--document` path by whoever starts it, which serves a board reached over a
cable, and `rivet-sysd` does not start it. A
first boot now stores what it was told; nothing yet applies it.~~

### In §8 — the wireless firmware requirement was unverified

**Changed 2026-08-22**, when the board was read; §8.1 is the reading. The
superseded phrase was ~~Unverified on this hardware~~.

### In §8 — a BusyBox DHCP stopgap was planned

**Changed 2026-08-26**, ruled by the maintainer. The superseded bullet read:

~~**A DHCP client is needed at Stage 1, before the Rust one exists.**
BusyBox's `udhcpc` is used as the initial implementation and is a Tier B
member per `ADR-001` section 4 — the Rust replacement is intended and
tracked, and the stopgap is not permitted to be forgotten.~~

It was dropped because Stage 1 had ended on 2026-08-22, so there was no longer
a thing being unblocked; the 2026-08-30 entry above records why `udhcpc` was
then adopted on its merits rather than as a stopgap.
