# ADR-031: Volumes

* **Status:** Accepted — ruled 2026-08-24. Four things are settled: a volume is
  identified by **GUID and never by device path** (§4), a volume holding
  programs is mounted **exec** (§5), **an absent volume means absent
  applications** (§6), and **the provisioning document declares volumes** (§7).
  Everything in §11 is open and carries a recommendation rather than a ruling.
* **Not implemented.** Nothing in this document has been built, and nothing in
  the tree reads a second block device. `AGENTS.md` §6.
* **Date:** 2026-08-24
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**Rivet assumes one storage device, and the assumption is written into the
record as a definite article.** `ADR-018` §4 rules that `/state` takes *"the
remainder of **the** device"*; its §10.1 repeats it; `ADR-030` §1 opens by
quoting it as *"the only writable filesystem on a Rivet machine"*. No section
of `ADR-018` contemplates a second one, and none of them is wrong — they were
written about a Raspberry Pi with an SD card in it, which is what existed.

Two machines the maintainer intends to run Rivet on have more than one:

* a small-form-factor x86 machine — the long-term target with no `tryboot` —
  carries a **477 GB NVMe
  and a 931 GB SATA SSD**;
* the Raspberry Pi plan puts an **SSD in an Argon One case beside the SD card**.

Neither is exotic and neither is served. A design that can only use the boot
medium has to put a terabyte of application data on the cheapest, slowest,
most-worn device in the machine, or not use the terabyte at all.

**The property this is for, as the maintainer put it: the system card can be
destroyed outright without anything of value being lost with it.**
*(`.rulings/` R-048.)*

That is the argument. `ADR-018` §2 already made the boot medium *replaceable* —
A/B for atomicity, read-only `/system` for wear — and stopped short of making it
**disposable**, because everything a machine had ever been given was on it. A
cheap A/B card is only genuinely cheap when losing it costs nothing.

---

## 2. Two nouns, and only one of them is new

| | **System device** | **Volume** |
| :-- | :-- | :-- |
| Is | exactly what `ADR-018` describes | a filesystem Rivet looks in for applications |
| How many | exactly one | one or more — `/state` is the built-in one |
| Holds | `p1`–`p5`: two boot slots, two system slots, `/state` | `app/<name>/…` and nothing else (§3.1) |
| Partition table | GPT, written by `cmd/rivet-image` | GPT, written elsewhere (§11 item 5) |
| Identified by | partition **number** on the disk the firmware booted (`ADR-018` §4.1) | **unique partition GUID** (§4) |
| Mounted by | PID 1, `p5` only (`ADR-017` §3.1.7) | above PID 1 (§11 item 2) |
| A/B, `tryboot`, first-boot growth | yes — `ADR-018` §5, §10 | **no, none of it** |
| Absent | the machine does not boot | the applications on it are not there (§6) |

**Nothing about the system device changes.** `ADR-018` §4's five partitions,
§5's A/B and `tryboot`, §7's wear measures, §8's `/state` recovery and §10's
first-boot growth are all statements about it, and this ADR disturbs none of
them.

### 2.1 Where `ADR-018` ends and this document begins

`ADR-011` forbids two documents being the authority for one fact, so the line is
drawn explicitly rather than inferred:

* **`ADR-018` is the authority for the system device** — its layout, its slots,
  the fact that `/state` is `p5`, `/state`'s mount flags, its filesystem, its
  growth, and its recovery. **This ADR never says anything about a partition
  table.**
* **`ADR-031` is the authority for what a volume is** — how one is named, how
  one is identified, how one is mounted, what it holds, and what happens when
  one is missing.
* `/state` is a volume. **That it is one is this document's; every other fact
  about it remains `ADR-018`'s.** In particular `/state`'s mount flags are
  `ADR-018` §4.1's and are *quoted* in §5 below rather than re-decided, and the
  fact that §5 arrives at the same flags for a data volume is a conclusion, not
  a second ruling.

The one sentence in `ADR-018` that this makes false is dealt with in §10.

---

## 3. An application and its data live together on whichever volume holds them

> **`<volume root>/app/<name>/` is the shape. `/state/app/<name>/` stops being a
> special case and becomes the built-in volume's instance of it.**

Everything under that directory is unchanged from `ADR-015` §7 and §5.1.1: the
version directories, the `current` symlink swapped by a `rename`, the signed
manifest kept beside the program (`ADR-015` §4.2.1), and `data/` at mode `0700`
owned by the application's derived uid. **The tree is identical; only its root
moves.**

`rivet_core::service::APP_ROOT` is `"/state/app"` today, a constant read by
`cmd/rivet-sysd`'s discovery, by `rivet_core::package::install`'s plan
and by the `/etc` projection. It becomes a function of a volume root, with
`/state` as the built-in one — see §8.

### 3.1 What a volume does *not* hold

Named, because the obvious next thought is to move more:

* **System extensions stay in `/state/ext`** (`ADR-018` §6). An extension is
  loop-mounted into the read-only `/system` tree; putting one on a removable
  device would make the shape of the base system depend on a device being
  present, which is the opposite of §6's property.
* **The configuration document, the secret store, the device identity and the
  journal stay on `/state`** — `ADR-004` §6, `ADR-020` §5 and §9.2, `ADR-017`
  §7. Every one of them is a fact about *this machine* rather than about a
  program, and `ADR-020` §7 binds the secret store to the board anyway, so
  moving it to a portable device would carry ciphertext nothing else can read.

**A volume holds applications. That is the whole scope**, and it is what keeps
`ADR-018` §4's *"reserved space for system use"* argument intact: the failure
that argument is about — application data filling the card until the machine
cannot be reconfigured — is exactly the failure a volume removes.

### 3.2 Why together, rather than program here and data there

This was argued and settled, and the losing side is the one with the better
sales pitch: put programs on the fast system device and data on the big one, so
each device does what it is good at.

| | **Together on one volume — RULED** | Program on the system device, data on a data volume |
| :-- | :-- | :-- |
| The volume is missing | **the application is not there** | **a running application with no data** |
| What the operator sees | an absence, named in the journal, at boot | a healthy service that silently recreates an empty store |
| Clobber the system card | lose nothing that matters — §1's property | lose every program, and orphan every data directory |
| Authorities for *where is `photo-index`* | one | two, which must agree |
| Trust in the program | the **signature** — `ADR-015` §4.2.1 | the signature, identically |
| The uid it runs as | derived from the name — `ADR-015` §5.1.1 | derived from the name, identically |
| Its data mode | `0700`, owned by that uid | `0700`, owned by that uid, identically |

**The first row decides it.** A running application with no data is the failure
class this project has now found three times and refused three times: `ADR-018`
§5.3.3's withheld commit, `ADR-030` §5.1's restored-but-dead secrets, and
`ADR-015` §5.1.1's uid that changes between boots — *a healthy-looking machine
that is wrong in a way nothing announces.* An empty store is worse than the
others, because the application will fill it, and by the time anybody notices
there are two divergent copies of the truth.

**Together, the failure is clean, obvious and safe:** the application simply is
not there, which is a thing an operator can see from the journal, from
`rivet get storage`, and from the service not existing.

**The last four rows are why the split buys nothing anyway.** Splitting looks
like a security or trust question and is not:

* **The disk is not an authority — the signature is.** `ADR-015` §4.2.1, ruled
  earlier on 2026-08-24, keeps each application's signed manifest beside it and
  re-verifies both halves at every launch. A program's *location* therefore says
  nothing whatever about whether it may run, and *"but it is on a removable
  device"* is a sentence with no consequence attached.
* **The uid drop is device-agnostic.** `ADR-015` §5.1.1 derives the uid from the
  name — deliberately, so that nothing has to be stored — which is precisely
  what makes a volume portable at all. Had uids been *allocated and recorded* in
  a file on `/state`, moving a volume to a replacement card would have silently
  changed every application's identity and every application would have failed
  to read its own files. That ruling was made for a different reason and this
  design depends on it.
* **`0700` is `0700` on any filesystem.** Nothing about the mode, the ownership
  or the `SO_PEERCRED` identity (`ADR-015` §5.3) knows which device it is on.

---

## 4. Identification is by GUID, never by device path

**`/dev/sda1` reorders between boots.** So does `/dev/sdb`, and on a Pi the USB
stack settles in whatever order it settles: an SSD in an Argon One case and a
USB stick somebody left in are two enumeration orders of the same hardware.
A design that mounts a path is a design that will one day mount the wrong disk
and report success.

Rivet already has the convention. `cmd/rivet-image`'s `disk.rs` gives the
system card a disk GUID and one partition GUID per partition, in a shape chosen
so that *"anyone reading `sgdisk -p` can see at a glance that it was chosen
rather than rolled"*:

```
DISK_GUID          FEEDFACE-0000-4000-8000-000000000000
partition_guid(n)  FEEDFACE-0000-4000-8000-{n:012X}      -- p5 is …000000000005
filesystem_uuid(n) FEEDFACE-0000-4000-8000-F{n:011X}     -- a separate namespace
```

### 4.1 Exactly how it extends — and the trap in extending it literally

**What is extended is the *kind* of identifier: a GPT unique partition GUID.
What is emphatically not extended is the `FEEDFACE` namespace.**

| | System device | Volume |
| :-- | :-- | :-- |
| Its GUIDs are | **fixed constants, identical on every card Rivet builds** | **unique, generated once when the volume is created** |
| Because | the image must be byte-identical between builds — `disk.rs` states the trade, and `ADR-015` §4's signature over an artifact that changes every build proves nothing | it is formatted in place on one machine and never distributed, so no signature is taken over it and there is nothing to make reproducible |
| So the GUID identifies | a **role** — *"whichever card is booted, this is p5"* | **this filesystem**, and no other |
| Resolved by | partition number on the disk the firmware booted (`ADR-018` §4.1) | reading GPTs and matching (§4.2) |

**The trap, recorded because reusing the existing constant is the obvious move
and it is wrong.** `partition_guid(5)` is the same string on every Rivet card
in the world. Two Rivet devices attached to one machine — which is a plausible
future on a box with two disks, and is exactly how a card would be recovered
after a failure — have **identical partition GUIDs**, so a system device cannot
be identified by GUID even in principle. A fixed identifier names a role; a role
cannot identify an instance. `disk.rs`'s own comment says the cost of the fixed
GUID *"falls where nobody is standing"*, and this is where somebody eventually
stands.

**A volume therefore gets an ordinary random version-4 GUID at format time**, and
its GPT *type* GUID stays the standard `0FC63DAF-…` Linux filesystem data that
`ADR-018` already uses. A private type GUID was considered and rejected for
`ADR-018` §7's reason for choosing `ext4` over `f2fs`: every recovery tool on
every other machine should understand this device without being taught, and
`ADR-018` §4's own refusal of the Discoverable Partitions Spec applies here too —
Rivet has nothing that discovers anything, so the marker would be decoration
with a compatibility cost.

**The GUID is written and compared as the full 36-character text form**,
case-insensitively, and **never truncated**. A short prefix collides far sooner
than intuition suggests, and the entire value of this identifier is that it
cannot be wrong.

*One trap for whoever implements the reader:* the first three fields of a GUID
are stored **little-endian** in a GPT entry and big-endian in the text form, so
bytes copied straight out of the table and formatted in order produce a
plausible, wrong UUID. This is the single most common defect in GPT parsers and
it fails silently — the string looks like a UUID and matches nothing.

### 4.2 How a GUID is resolved without udev

`ADR-018` §4.1 appears to forbid this and does not. Its words:

> `/dev/disk/by-partlabel/` is **udev's**, and `ADR-005` §2.1 rules that Rivet
> has no udev, so nothing on a board creates those symlinks. Resolving a label
> without udev means parsing the GPT out of the raw disk **inside the process
> that must not fail**.

**The load-bearing clause is the last one.** That paragraph is about PID 1
mounting `p5`, and `ADR-017` §3.1.5 is a complete and deliberately short list.
Volume attachment is not on it and must not be added to it — the same answer
`ADR-018` §10.3 gave when asked whether PID 1 should grow `/state`: *"It must
not be PID 1's job."*

So:

1. **PID 1's job is unchanged.** It mounts `p5` by number and knows nothing
   about volumes.
2. **A volume is attached above PID 1**, by a process that is allowed to fail,
   which lists block devices from `/sys/block` (devtmpfs has already made the
   nodes — `ADR-005` §2.1), reads each device's GPT, and matches unique
   partition GUIDs against the ones the document names.
3. **A failure there degrades the machine and never makes a boot
   unconfirmable** — `ADR-018` §8's argument reaching one device further out: a
   rollback cannot repair a device an update never touched, so withholding the
   commit would alternate between two healthy images for ever without addressing
   the fault.

**The GPT reader this needs already exists in the tree.** `ADR-018` §10.7 chose
Rivet's own Rust over `sgdisk` for the first-boot grow, and
`rivet-adapter-image`'s `grow` module carries a UEFI 2.10 header parser and a
partition-entry parser with `entry_at::{TYPE_GUID, FIRST_LBA, LAST_LBA, NAME}`.
**It is missing exactly one constant** — the unique GUID at entry offset 16 —
and a read-only path that enumerates devices instead of taking one. That is the
whole implementation cost of §4, and it is an accident worth noticing: the
mechanism this ADR needs was built the same day for a different reason.

### 4.3 The honesty half

`ADR-011` §2 part two: *the system says which one, at the moment the answer is
needed.* An operator told to identify a volume by GUID needs a way to read the
GUID off the machine, or the rule is unobeyable and they will guess. **`rivet
get storage` reports it** — §9.

---

## 5. A volume holding programs is mounted exec

**Mount options for every volume, unconditionally:**

```
noatime, nosuid, nodev            -- and deliberately NOT noexec
```

These are the same four decisions `ADR-018` §4.1 made for `/state`, reached
independently for a device that is not a card, and `ADR-018` §4.1 remains the
authority for `/state`'s own flags (§2.1).

**`noexec` is stated explicitly because a later reader will wonder, and because
a data-only design would have wanted the opposite.** The phrase *"data volume"*
invites `noexec`; `ADR-018` §4.1 says of `/state` that `noexec` *"is the flag one
reaches for on a writable partition"* and that *"setting it should be a ruling
with a stated consequence, not a tidy-up."* Here is the ruling and here is the
consequence:

> **§3 puts the program and its data on the same volume. A volume mounted
> `noexec` is therefore a volume on which no application can ever start.**

And it would fail in the worst available place. Not at mount time, where the
message could name the flag, but at `execve` inside the supervisor, as `EACCES`
against a binary that is present, correctly signed, correctly owned, and
verified — an error whose text points at permissions on a file whose permissions
are right.

**The other three are not negotiable per volume, and each buys something:**

| Flag | Why, on a device that is not the boot card |
| :-- | :-- |
| `nosuid` | a volume may have been written by another machine and is the one filesystem on a Rivet box whose contents Rivet did not put there. Nothing in the design uses setuid — `ADR-015` §5.2 passes descriptors rather than granting `CAP_NET_BIND_SERVICE`, and the image contains no setuid binary — so a set bit here is attack surface with no legitimate use |
| `nodev` | the same argument, and stronger: a device node on a data volume has no honest purpose at all |
| `noatime` | `ADR-018` §7's reason where the volume is flash, and harmless where it is not. Unconditional rather than a per-volume key, because a key that is always set the same way is a key nobody should have to set |

**A volume is mounted read-write.** A read-only volume is not ruled here — §11
item 9.

---

## 6. An absent volume means absent applications

This is the safety property the whole shape turns on, and it is the reason §3.2
chose "together". It is stated as a rule rather than as a behaviour:

> **A volume the document names and the machine cannot find is reported, and
> every application on it is simply not there.**
>
> It is never started against an empty directory. Nothing beneath its mount
> point is created, written or read. There is no fallback to another volume and
> no partial start.

### 6.1 The specific hazard, which is a directory that gets created

`cmd/rivet-sysd`'s discovery today handles `ENOENT` on `/state/app` by
saying, at `Info`:

```
no applications are installed on this machine
```

**That is correct today and becomes a lie the moment volumes exist.** On a
single-device machine, no `/state/app` really does mean nothing is installed. On
a machine whose document declares a volume, `ENOENT` at that volume's app root
means *the device is missing* — and reporting a missing terabyte as "nothing is
installed", at `Info`, is the silent-wrong-answer failure this project keeps
paying to discover.

**So the two cases are distinguished by whether the volume was attached, not by
whether a directory exists.** A directory listing cannot tell them apart and must
not be asked to.

The second half matters as much. If the mount fails and anything then creates
`<mount point>/app/photo-index/data/`, the machine has **quietly begun a second
copy of the application's data on the system device** — which is precisely the
failure §3.2 chose "together" to avoid, arriving through the back door, and it
would be discovered when the real volume came back and the two disagreed.
Therefore: the mount point may be created, the mount is attempted, and **if the
mount fails, nothing beneath it is ever created or read.**

### 6.2 What is reported, and how loudly

* **At `Severity::Error`, once per absent volume**, naming the volume's name and
  its GUID, and saying in terms that the applications on it are not running. Not
  `Warn`: a declared device that is not there is a fault, and `ADR-015` §6.1's
  standard is that a machine must never report a capability it does not have.
* **The machine keeps running.** One absent volume is a failed volume, not a
  broken system — `ADR-017` §3.1.4's principle, the same one `ADR-015` §4.2
  applies to an application whose signature fails: refuse that one thing, loudly,
  and never take the machine down over it. A board nobody can log into cannot be
  repaired.
* **The boot is still confirmed** (`ADR-018` §5.1 item 4). Withholding it would
  roll back to an image that is not the problem and would alternate for ever —
  `ADR-018` §8, again.
* **`rivet get storage` shows the absence** — §9. The journal says it once at
  boot; the operator needs to be able to ask afterwards.

### 6.3 The property, in one sentence

> **An application is present or absent. It is never present-and-empty.**

---

## 7. Provisioning declares volumes; the machine never guesses

`ADR-004` §6 makes `rivet.toml` on the FAT boot partition the first-boot
channel, and has now done this job three times: the wireless credential (§6), the
`[network]` table (§6.1). Both amendments
close with the same sentence — *nothing new is built: the file, the mount, the
parser, the store and the refusal path all existed.* **This is the fourth job for
the same mechanism**, and the sentence holds a fourth time.

### 7.1 The schema

`ADR-007` §7 gains a section. One sub-table per volume, in the shape `[services]`
and `[network]` already use:

```toml
[volumes.media]
guid = "9f2c1e40-7b3a-4c58-9d61-2ab7e0c4f183"
```

* **`guid` is required**, and is the full 36-character form (§4.1).
* **The volume's name is a path component**, so it is refused by the same rule
  that refuses an application version — `rivet_core::package::install::
  safe_component`, whose most valuable clause is the one that catches names which
  already *mean* something (`ADR-015` §7.1.1). It may additionally not be
  `state`: `/state` is the built-in volume and a second thing called `state`
  would be two answers to one name.
* **There is no `device` key, no `path` key and no `label` key.** Adding one
  would be the second authority `ADR-011` forbids, and it would be the one a
  tired operator reaches for.

### 7.2 Refusing an unknown key is what makes this safe

`ADR-007` §3.6 rules that the system never accepts an input it will discard, and
`ADR-018` §5.3.3 turned that into an experiment: an unknown key in `p2`'s
`rivet.toml` produced a completely healthy board whose *only* fault was a line
that did not appear. The same mechanism does two things here:

1. **A machine cannot acquire a volume by a typo.** `[volume.media]` is refused
   at parse time and named, rather than being an empty set of volumes.
2. **A machine running an older image refuses a document that declares volumes**,
   rather than coming up healthy with half its applications missing and saying
   nothing. That is the direction that matters: the alternative — ignore the
   unknown table — produces exactly §6's failure with no message at all.

### 7.3 Auto-discovery is rejected

The machine could mount every filesystem it finds and run whatever is on it. It
must not.

| | **The document names volumes — RULED** | The machine mounts what it finds |
| :-- | :-- | :-- |
| A disk from another machine is plugged in | ignored | mounted, and its applications **started** |
| Two disks holding the same application | a declared conflict, refusable in advance | a race decided by enumeration order |
| The set of applications on the machine | described by the document | **discovered**, so the document no longer describes the machine |
| Authorities for what software runs here | one | the document **and** the hardware |
| `ADR-007`'s guarantee | intact | broken |

**The third row decides it.** `ADR-015` §2.1 keeps the desired-state guarantee
alive across applications on exactly one argument: *"the set of installed
applications is part of the document. Nothing is installed imperatively."* A
machine that runs what it finds has installation by side-channel, which is what
`ADR-011` exists to prevent — and the first row is the shape of every autorun
vulnerability ever written.

### 7.4 Two consequences carried over from `ADR-004` §6

* **A GUID is not a secret and is not removed from the provisioning file.**
  `ADR-004` §6 step 3 removes the wireless passphrase; §6.1 keeps the address and
  §6.2 keeps the public key, both on the argument that a provisioning file which
  still describes the machine's world is a card that can be re-imaged without
  being told twice. A volume GUID is the strongest case for that yet: **it is the
  thing that makes a replacement card find the data again**, and it is §1's
  property made operational.
* **Seeding never overwrites** (`ADR-004` §6.1 item 3). The boot partition seeds
  `/state/config/machine.toml` on a first boot only; thereafter the stored
  document is the authority, and a boot partition that could reassert itself
  would make the FAT partition authoritative for configuration.

### 7.5 Declaring a volume does not create one

`ADR-007` §7.5: *a schema says what may be asked for; an apply says what can be
done.* A declared volume whose GUID is nowhere on the machine is §6's absent
volume — **not an instruction to format something.** Formatting is an operator
action against a named device with a confirmation, and it is §11 item 5.

---

## 8. What this costs the supervisor

`ADR-017` §4's second bullet — *"Application services come from the
application's own manifest; installing the application is what makes the service
exist"* — is unchanged in substance and changes in every detail of how it is
performed. Discovery goes from *look in `/state/app`* to *look across volumes*,
and the costs are named rather than discovered:

1. **`rivet_core::service::APP_ROOT` stops being a constant.** It is
   `"/state/app"` today, read by the supervisor's discovery, by
   `rivet_core::package::install`'s plan, and through `service::app_dir` by the
   `/etc` projection. It becomes a function of a volume root. **The built-in
   volume's root stays literally `/state`**, so `/state/app` is byte-for-byte
   where it was and no machine has anything to migrate — `ADR-015` §2's table row
   stays true of it.

   **BUILT 2026-08-25.** `service::app_root(volume)` is the function and
   `service::BUILT_IN_VOLUME` is `/state`. Three things fell out of it that were
   not in the paragraph above:

   * **A `&'static str` spelling of `/state/app` had to survive**, because three
     `const` tables cannot call a function — `generated`'s catalogue,
     `operation::edit`'s reserved list, and `service`'s own tests. `APP_ROOT`
     stayed, and a tripwire asserts it equals `app_root(BUILT_IN_VOLUME)`. Two
     spellings of one path is what `ADR-011` is about; the test is the whole of
     what keeps them one.
   * **`rivet_core::etc::ApplicationAccount` grew a `volume` field** — §8 item
     4 arriving early, because `passwd`'s home field is `service::data_dir`'s
     answer and that stopped being a function of the name alone.
   * **`cmd/rivet-sysd`'s `ServiceDef` grew `volume: Option<String>`**, `None`
     for an `Origin::Image` service and `Some` for an application. It is carried
     rather than recomputed because four separate places need the application's
     paths, and re-deriving the volume in each is four chances to disagree about
     where an application lives. `finish_uninstalls` **refuses** to remove an
     application whose volume it cannot name rather than guessing `/state` —
     a removal pointed at a path nothing derived is the worst available
     failure.

   **What it does not do:** choose. Every caller passes the built-in volume,
   because choosing needs the attached set and §11 items 1, 2 and 8 are unruled.
   The value of the change is that the diff now marks every place a choice will
   have to be made.
2. **`ENOENT` no longer means "nothing installed"** — §6.1. This is the one that
   will be got wrong if it is not written down. **Still true and still not
   handled** as of 2026-08-25: `discover` treats `ENOENT` as *nothing installed*,
   which is safe only because the built-in volume is always attached — the
   machine is running off it.
3. **A failure is per volume rather than one failure.** The supervisor's existing
   argument — *"a fault here never stops the boot… every failure removes that
   application and leaves everything else running, loudly"* — generalises without
   change: an unreadable volume removes that volume's applications.
4. **The `/etc` projection walks more than one directory.**
   `rivet_core::etc::EtcInputs::applications` is a list of names, and the list
   is now assembled across every attached volume. A uid collision across two
   volumes is a collision exactly as it is within one — `ADR-015` §5.1.1's
   refusal is unchanged, except for the case §11 item 3 opens.
5. **Every directory from `/` down to the binary must be searchable by the
   application's uid.** `ADR-015` §5.1.1's third bullet now includes **the mount
   point and the volume's root directory**, and neither is made by the installer —
   they are made by whoever mounts, or they arrived with a filesystem formatted on
   another machine. **A volume that arrives mode `0700` root-owned makes every
   application on it fail with `EACCES` after the uid drop**, at a moment nobody
   is watching, which is `ADR-015` §5.1.1's own stated nightmare. Whoever mounts
   checks the traversability and reports it.

### 8.1 Where the code goes

`AGENTS.md` §3's test, applied once so it is not asked four times:

* **Reading a GPT and enumerating block devices is an adapter's own I/O seam** —
  `Errno`, block devices, sector reads. It belongs beside
  `rivet-adapter-image`'s existing GPT code (§4.2) or in
  `rivet-adapter-system`, which already owns the mount table.
* **Deciding whether the declared set matches the found set is pure**, and it is
  `rivet-core`'s: given a list of declared volumes and a list of
  (device, GUID) pairs, produce the attached set, the absent set and any
  duplicate-GUID refusal. No I/O, no platform types, testable with no device —
  which is the cheap standing check `AGENTS.md` §10.1 names.

---

## 9. Volumes are visible in `rivet get storage`

`ADR-005` §4.1.3 ruled `get storage` on the argument that *"free space is a
fact"*. `rivet_core::operation::storage` reports one record per mounted
filesystem with a `holds` field — *which of Rivet's own named locations this
filesystem is the one holding* — computed by longest mount point over
`named_locations()`, a **compile-time array of four constants**: `/system`,
`/state`, `/state/config`, `/state/log`.

**It cannot answer this as it stands, because a volume's mount point is not a
compile-time constant.** Three changes, and the third is the one that matters:

1. **`named_locations()` becomes a function of the machine's attached volumes**
   rather than a constant array. A volume's root is a Rivet-named location in
   exactly the sense `/state` is, and `holds` is where a reader already looks to
   find out which filesystem is carrying what.
2. **Each filesystem record gains `guid`** — the unique partition GUID where one
   can be read, absent where it cannot. This is §4.3's honesty half: without it,
   an operator told to identify a volume by GUID has no way to obtain one, and
   the optional-field convention the module already documents (`select guid?`)
   covers the pseudo-filesystems that have none.
3. **The result gains a top-level `volumes` list — one record per *declared*
   volume**: name, GUID, `attached`, and mount point when attached. A
   declared-but-absent volume has **no filesystem row at all**, so no
   per-filesystem field can report it, and the module's own rule about
   `unreadable` is the precedent, in its own words:

   > an answer that omitted one would say THERE IS NO SUCH FILESYSTEM, which is
   > the one thing this must not say falsely (`AGENTS.md` §5).

   An absent volume omitted from the answer says exactly that, about a device the
   operator declared and can see in their hand.

Nothing is filtered — the module's standing rule, and unchanged.

---

## 10. What this makes false in the existing record

`AGENTS.md` §2 rule 3: supersede in place, never silently. This ADR does not edit
`ADR-018` or `ADR-030`; it names what has to change there, and §12 is the list.

1. **`ADR-018` §4's *"the remainder of **the** device"* is not overturned — it is
   scoped.** It is a statement about the **system** device and remains exactly
   true of it. The same goes for §10.1's repetition and for §10.2's first-boot
   growth, which grows `p5` and nothing else.
2. **`ADR-018` §7's and `ADR-030` §1's *"the only writable filesystem on a
   Rivet machine"* becomes FALSE** on a machine with a volume, and this is the
   one genuine contradiction. Two consequences follow rather than one:
   * `ADR-018` §2's *"the wear risk is not spread across the card, it is
     concentrated entirely in `/state`"* is still true **of the card**, and a
     volume is a second wear surface for which none of §7's measures has been
     ruled. §5's `noatime` is the only one that carries over automatically.
   * `ADR-030` §1's opening paragraph derives the whole existence of that ADR
     from the quoted claim. Its conclusion survives — an A/B rollback still
     cannot repair anything outside `/system` — but the sentence it is built on
     needs the same scoping as item 1.
3. **`ADR-030` §2's *reason* for excluding application data no longer holds.** It
   reads: *"`ADR-015` has no data model yet, and inventing one here would be the
   second authority `ADR-011` forbids."* **This ADR is that data model**, at
   least as far as location goes: an application's data is
   `<volume>/app/<name>/data`, on a named, identified device. The exclusion is
   still right (§11 item 6) and its stated justification is now stale, which is
   the more dangerous of the two failures because it reads as settled.
4. **`ADR-018` §4.1's *"the device is named by number, not by `PARTLABEL`"* is
   NOT contradicted**, and §4.2 above is careful to say why: that argument is
   about the process that must not fail, and volume attachment is not that
   process. An agent reading §4.1 in isolation will conclude this ADR overturns
   it; it does not.
5. **`cmd/rivet-image`'s fixed GUIDs are a role namespace and must not be
   extended to volumes** — §4.1. Two Rivet system devices in one machine carry
   identical partition GUIDs. Recorded here as well as in §4.1 because reusing
   `partition_guid` is the obvious implementation and it silently produces an
   identifier that identifies nothing.
6. **`ADR-015` §2's table row *"Application | `/state/app/<name>`"* becomes one
   instance of a shape** rather than the definition. It stays literally true of
   the built-in volume, which is why nothing has to move.

---

## 11. Open

Nine questions this ADR surfaced and does not rule. Each carries a
recommendation; none is a decision.

1. **How a volume is added to a running machine.** There is no udev, no hotplug
   and no mount policy (`ADR-005` §2.1) — and `ADR-030` §4 already declined USB
   mass storage as a backup destination on exactly that list. *Recommendation:
   it is not. A declared volume is attached at boot, and adding one is a
   document change and a restart.* That is cheap on this hardware, adds no
   mechanism, and keeps the attached set a function of one document read at one
   moment rather than of what has been plugged in since. Revisit if a fleet ever
   needs it.
2. **Which process attaches volumes.** *Recommendation:
   `cmd/rivet-sysd`, before it converges anything.* `ADR-017` §4.3 is the
   precedent — it already reads `/state/config/machine.toml` itself before
   converging — and mounting must precede discovery, which is the supervisor's.
   **Not PID 1**, per §4.2. The honest cost is that the supervisor grows a GPT
   reader and a block-device enumeration, which is new surface in the process
   `ADR-017` §3.1 kept small on purpose.
3. **Two volumes holding an application of the same name.** The uid derivation
   gives them one uid, so `ADR-015` §5.1.1's collision refusal already applies —
   but its tie-break is *"deterministically by name order"* and here the names
   are equal. *Recommendation: refuse **both**, name both volumes in the journal,
   and start neither.* Starting the first found makes the outcome depend on
   enumeration order; starting the one on `/state` makes the system device
   authoritative for something §3 just took away from it. Refusing both is the
   only outcome that is identical on every boot and cannot be produced by
   somebody plugging in a disk.
4. **Whether a volume can be removed, and what that means for its data.**
   *Recommendation: removing a declaration stops that volume's applications and
   touches no bytes.* Nothing is erased by a configuration change; erasing is a
   separate operator action against a named device with a confirmation
   (`ADR-012` §7.1, `AGENTS.md` §10.4). The asymmetry is deliberate — a
   configuration edit that destroys a terabyte is not a shape this project should
   have.
5. **Formatting and creating a volume.** Nothing in Rivet makes a filesystem
   today; `cmd/rivet-image` does it on a host under WSL. *Recommendation:
   `rivet apply volume create <device> <name> confirm`, refusing outright any
   device carrying a partition table Rivet did not write, with `AGENTS.md`
   §10.4's destructive-command rule applying in full.* Until it exists a volume
   is made on another machine and only declared here — which is enough to prove
   everything else in this document, and is the cheaper first step.
6. **`ADR-030`'s scope, now that application data has a location.** See §10 item
   3. *Recommendation: application data stays out of scope, and `ADR-030` §2's
   **reason** is rewritten rather than its ruling* — the honest reason is that it
   can be arbitrarily large and now lives on a device chosen precisely so it
   survives the card, which is a better argument than the one it replaces. The
   backup's own scope — the document and the secret store — is unchanged, and
   `ADR-030` §5's hardware binding is untouched by any of this.
7. **Whether `/state` and a data volume differ in any way that matters.** As
   volumes, **no**: same tree shape, same mount flags, same rule that an absent
   one means absent applications. Three differences are not about being a volume
   — `/state` also carries the configuration, the secrets, the journal and the
   extensions (§3.1); `/state` is grown at first boot (`ADR-018` §10) and a volume
   is not; `/state` is resolved by partition number and a volume by GUID (§4).
   *Recommendation: keep the built-in volume's root literally `/state` rather
   than relocating it under a uniform mount point.* A machine with no data volume
   then has a layout byte-identical to today's, and there is no migration to get
   wrong. The cost is one visible irregularity, and it is worth it.
8. **Where an additional volume is mounted — RULED 2026-08-25:
   `/volumes/<name>`.** The
   maintainer took the recommendation as it stood. The reasoning it was made on
   is now this document's, and is repeated here because a ruling that keeps only
   the answer loses the part that constrains the next question: **plural**, so
   the path cannot be confused with a single well-known location; and **under
   the root rather than under `/state`**, because a volume nested inside the
   filesystem it exists to be independent of is a trap.

   **What this unblocks and what it does not.** §12's adapter and supervisor
   rows may now be written — the mount point is a constant something in the tree
   is allowed to spell. Items 1, 2 and 5 are still open, and they decide how a
   machine gains a volume at all, so a declared volume still has no code path
   that attaches it.
9. **A read-only volume, and wear on a volume that is not a card.** *No, and
   nothing, respectively.* A read-only flag is a key with no requester and §5 is
   one decision shorter without it — though if it is ever added it must refuse at
   **mount** rather than let an application fail at its first write, which is §3.2's
   failure by another route. `ADR-018` §7's wear measures were ruled against an SD
   card; an NVMe is a different device, and `f2fs` remains `ADR-018` §9 item 4's
   question rather than being asked twice here.

---

## 12. What has to change elsewhere, and three rows of it are not done

Listed so that the gap between this document and the tree is legible rather than
inferred. **The heading counted *none* and then *most* before it counted three**,
which is the fact this section exists to keep legible.

**Ten of the thirteen rows below are written and dated `BUILT`. Three are not**,
and everything buildable without a ruling is built.

**Item 8 was ruled on 2026-08-25 — `/volumes/<name>`** — so the mount point is
no longer what blocks these rows. **Items 1, 2 and 5 are still open**,
and they are the ones that decide how a machine gains a volume at all: a mount
point nothing mounts to is still a constant rather than a feature.

| Where | What |
| :-- | :-- |
| `ADR-018` §4, §7 | **BUILT** — §4's table scoped to the system device 2026-08-24; §7's wear measures scoped 2026-08-25 |
| `ADR-018` §4.1 | **BUILT 2026-08-25** — the by-number rule now says it is about PID 1 and points at §4.2 |
| `ADR-030` §1, §2 | **BUILT 2026-08-24** — the quoted claim is struck in §1 and the stale reason superseded in §2's table |
| `ADR-015` §2 | **BUILT 2026-08-25** — the row and the paragraph under it |
| `ADR-017` §4 | **BUILT 2026-08-25** — §4 now carries the five costs and says which three are unbuilt |
| `ADR-007` §7 | **BUILT 2026-08-25** — the schema is `ADR-007` §7.8 |
| `ADR-005` §4.1.3 / `get storage` | `guid`, the `volumes` list, and a dynamic `holds` — §9 |
| `docs/PLAN.md` | **BUILT 2026-08-25** — `PLAN.md` §18, with §18.6 listing the four rulings that block the rest |
| `rivet_core::service::APP_ROOT` | **BUILT 2026-08-25** — it is `service::app_root(volume)`, and `service::BUILT_IN_VOLUME` is `/state`. Every caller passes the built-in volume; the diff is where a choice will have to be made |
| `rivet-adapter-image`'s `grow` | `entry_at::UNIQUE_GUID = 16`, and a read-only enumerate-and-match path — §4.2 |
| `cmd/rivet-sysd` | attach before converge; `ENOENT` per volume; per-volume failure — §6.1, §8, §11 item 2 |
| `rivet-config` | **BUILT 2026-08-25** — `src/volumes.rs`, the parser, the canonical rendering and `get schema`. Nothing reads the declarations yet |
