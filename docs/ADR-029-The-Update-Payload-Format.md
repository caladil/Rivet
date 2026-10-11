# ADR-029: The Update Payload Format

* **Status:** Accepted
* **Date:** 2026-08-23
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-018` §5.2 describes an update as a *flow*: write the inactive slot, verify,
`tryboot`, confirm or revert. It never says what arrives over the wire.
`ADR-027` §4 reserved a `<kind>` token for that artifact and **declined to name
it**, correctly, on the grounds that naming a format nobody has designed is the
`AGENTS.md` §7 failure. This ADR designs it, which makes `ADR-027` §9 item 1
answerable.

It is its own file rather than an `ADR-018` §11 because it is one decision, and
because `ADR-027` and `ADR-028` were split out of `ADR-018` on the same
reasoning.

**Four things were ruled by the maintainer on 2026-08-23** and are recorded in
their own sections: the signature scheme (§7.1), that install and arm stay one
operation (§9.1), that firmware blobs are always carried (§6.2), and the digest
algorithm (§7.2).

---

## 2. The decision

| Question | Ruling | Where |
| :-- | :-- | :-- |
| One file or several | **One.** The two halves land in different *kinds* of place; the asymmetry is preserved in the manifest, not split across artifacts | §3, §10 |
| What the signature covers | A **fixed 4096-byte header at offset 0**, signed as one unit, holding every member's digest. Content is covered transitively | §5 |
| Verification order | **Entirely before any write.** Five integer comparisons stand between the file and the signature check | §5 |
| Whole filesystem or delta | **Whole.** Per-member digests give member-granularity skipping for free, with no scratch space | §6 |
| Firmware blobs | **Always carried** | §6.2 |
| `<kind>` token | **`update`** | §8 |
| Suffix | **`.rvt`**, one for all artifact kinds; `<kind>` distinguishes | §8 |
| Compression | **Inside the container, per member**, digests over the *stored* bytes | §5.1 |
| Signature scheme | **Ed25519, raw** — no X.509, no ASN.1, no DER | §7.1 |
| Digest | **BLAKE3**, pure-Rust | §7.2 |
| One format for three artifact kinds | **One container, one verifier**; `kind` distinguishes | §11 |

**Built 2026-08-24 — §13** for the
container, **§14** for the installer and **§15** for the builder, which are the
itemised accounts of what exists, what each implementation had to decide that
this ADR did not, and what is still open. **§15 is the one to read if you are
asking whether the chain is closed**, because until it landed the installer could
install a payload nothing could produce. This document said nothing in it was
implemented until that day, which is the class of claim `AGENTS.md` §7
is about in both directions — and §13.4's *"nothing
writes to a slot in shipped code"* went stale within hours of being written,
which is the same failure one layer down.

---

## 3. The layout

```
byte 0                                                          byte 4096
+---------------------------------------------------------------+
| HEADER — fixed 4096 bytes, signed as one unit                  |
|  0   8  magic          "RIVETPKG"                              |
|  8   4  format_version u32 = 2      (was 1 — see §3.1)         |
| 12   4  header_bytes   u32 = 4096   (checked, never obeyed)    |
| 16   4  sig_alg        u32                                     |
| 20   4  digest_alg     u32                                     |
| 24  32  key_id         digest of the public key — ADR-028 §5   |
|  ----- nothing below here is parsed until step 7 of §5 -----   |
| 56   4  member_count   u32, <= 40                              |
| 60   4  flags                                                  |
| 64  32  product        "rivet" | "metrics-collector"         |
| 96  32  version        "0.2.0" | "0.0.1-g7b84d4b"              |
|128  16  arch           "aarch64"                               |
|144  16  platform       "rpi4"                                  |
|160  16  kind           "update"                                |
|176   8  build_epoch    i64, informational, NEVER compared      |
|184 ...  member records, 96 bytes each                          |
|4024  8  reserved       (was 24 bytes before §3.1)              |
|4032 64  signature over bytes [0 .. 4032)                       |
+---------------------------------------------------------------+
| MEMBER 0 stored bytes, MEMBER 1 stored bytes, ...              |
+---------------------------------------------------------------+
```

**The magic is eight printable bytes, `RIVETPKG`**
(`rivet_core::package::format::MAGIC`), so a hex dump names the file in full.
It is the first thing a reader compares, which is also what catches a
text-mode transfer that mangled the bytes. The trust store's own file uses
`RIVETTRS` on the same rule, so neither can be mistaken for the other.

A member record is 96 bytes: `name` (32, ASCII, NUL-padded), `digest` (32, over
the **stored** bytes), `target_class` (1), `slot_variance` (1), `compression`
(1), reserved (1), `offset` (8), `stored_len` (8), `plain_len` (8), reserved (4).

**`target_class` is `PayloadTarget`** — deliberately the same two cases in the
same order. `rivet-adapter-image`'s `SlotPayload` already types the asymmetry
between a whole system partition and a directory on a FAT filesystem; the format
restates it rather than flattening it.

**`slot_variance` exists because exactly one boot file differs between slots**,
and that was measured rather than assumed: extracting `p1` and `p2` from a built
image and diffing them, **`cmdline.txt` is the only file that differs**
(`root=PARTLABEL=system-a` versus `-b`). Everything else is byte-identical. So a
payload carries `cmdline.txt` twice — **136 bytes** of duplication — and the
alternative, generating it on the device, would stand a second authority over
the kernel command line beside `cmd/rivet-image/src/boot.rs` (`ADR-011`).

---


### 3.1 `format_version` is 2 since 2026-08-24, because `product` was too small for `ADR-015`'s own example

`product` was 16 bytes. `metrics-collector` — the worked example in `ADR-015`
§5.4 — is 17 characters, so `cmd/cargo-rivet` **refused to package the ADR's
own example.** Refusing was right (truncating silently renames somebody else's
application, which is `AGENTS.md` §3's whole subject), but the refusal was the
format's fault rather than the name's.

**The maintainer ruled it grown while growing it was still free** — grow it now,
while that is still possible. *(`.rulings/` R-046.)* The argument was about
timing rather than size:

> Nothing had ever verified an `.rvt` on a Rivet, because no image carried a
> trust store (`ADR-028` §5.7). There were **zero deployed artifacts**, so no
> compatibility branch is owed to anybody — and the window closes on its own the
> day the trust store ships.

`product` is 32 bytes. Every field after it moved by 16, the member table with
them, and `format_version` is therefore **2**. **Version 1 is refused**, not
supported: a version-1 header read by this parser would find `version` where
`product` ends. `package::tests::step_3_an_unimplemented_format_version_is_refused`
now names `1` in its list, and that is the point of the test rather than a
detail of it.

**The header is now nearly full and this is said out loud.** Reserved space went
from 24 bytes to **8** — `184 + 40 × 96 = 4024`, and the signature starts at
4032. That is not a cliff: `header_bytes` is itself a field at offset 12, so a
future version may declare a larger header and a reader can tell which one it is
holding. That designed escape hatch is what made spending 16 of the 24 bytes
affordable. **The next field that does not fit in 8 bytes is a header-size
decision, not a squeeze.**

## 4. Two names are reserved, and a payload containing either is refused

**`autoboot.txt` and `rivet.toml` are never members.**

This is the sharpest thing the design pass found, and the record did not contain
it. It is a conjunction of three facts, each innocuous alone:

1. `rivet_core::state::AUTOBOOT_PARTITION` is **1**, and
   `PiFirmware::default_slot_record` returns `p1` **whichever slot is running** —
   proven on hardware 2026-08-23 and asserted twice in `slots.rs`'s tests.
2. `PiFirmware::payload(SlotId(1))` returns
   `DirectoryOnPartition { device: "/dev/mmcblk0p1", path: "/" }`.
3. `cmd/rivet-image`'s `boot::contents` writes `autoboot.txt` onto **both**
   boot partitions when building a card.

**So when slot A is the inactive slot, the payload's boot target is the
filesystem holding the only record of which slot the firmware boots.** A payload
carrying `autoboot.txt` would, on an update to slot A, overwrite the commit
record with a fresh-card default — and every subsequent boot would go to slot A.
The machine's actual default would have been silently reset by an update that
reported success.

`rivet.toml` is refused for the same class of reason: `ADR-004` §6 has PID 1
*mutate* it (removing the credential after first boot) and `ADR-018` §8 makes it
the recovery path for a damaged `/state`. An update restoring the blank template
destroys both.

**The refusal is of the whole payload, not of the member.** Silently skipping
would let a future builder believe it had shipped something it had not.

**This is the port's job, not the installer's.** `BootSlots` gains an operation
naming what a platform reserves on a payload target — on the Pi that is a pair
of filenames; on a UEFI machine the equivalent object is an EFI variable and is
not a file at all, which is exactly why it belongs behind the port.

---

## 5. What is signed, and the order that makes it safe

`ADR-028` §2's last row is the binding constraint: **verification is the most
security-sensitive parser on the machine, and everything upstream of it parses
attacker-controlled bytes.** The format is shaped so that "upstream of it" is as
close to empty as a format permits.

Before the signature is checked, an implementation performs exactly this:

1. file length ≥ 4096 — else refuse
2. `bytes[0..8] == magic` — else refuse
3. `format_version == 1` — else refuse
4. `header_bytes == 4096`, **checked against the constant and never used to size
   a read** — else refuse
5. `sig_alg` and `digest_alg` are ones this build implements — else refuse
6. `key_id` is in the trust set (`ADR-028` §5.2's root set from `/system`, plus
   countersigned configuration keys) — else refuse
7. verify the signature over `[0 .. 4032)`

**Five integer comparisons and a table lookup.** No length field is trusted, no
loop is bounded by attacker data, nothing is allocated. Only after step 7 is the
manifest region parsed — and by then its bytes are ours, so `member_count`,
`offset` and `stored_len` are authenticated before they are believed.

Then, per member:

8. read `stored_len` bytes at `offset`, digest, compare against the signed
   digest; **refuse on mismatch, having written nothing further**
9. only now hand those bytes to the decompressor

**`ADR-015` §4.1's requirement is met and is worth stating explicitly**, because
a reader who does not see it spelled out will conclude the body is unsigned:
binary and manifest are signed as one unit. Here the signature covers the header
alone and the header carries every member's digest, so the content is covered
transitively.

### 5.1 Why compression is inside the container

If compression were outside — `.rvt.xz` — **the decompressor would be the thing
upstream of the verifier**, and it would be a large codebase processing bytes
straight off the wire. If digests were taken over the *plain* bytes, the same
would be true one layer in. As designed, the decompressor only ever sees bytes
the signer produced.

`compression = none` is a legal per-member value and the builder uses it
whenever the saving is under about 5%. Two measured cases: `cmdline.txt`
compresses *larger* than it is (192 bytes stored against 136 plain), and
`kernel8.img` compresses by **0.6%** because it is already a compressed kernel —
storing it plain costs 61 KB of payload and saves decompressing 10 MB of LZMA on
a core with no crypto extensions.

---

## 6. Whole filesystem, not a delta — argued on measurement

Measured 2026-08-23 against a built `out/` image (2 GiB, five partitions, the
`ext4` stopgap of `ADR-018` §3.1):

| Member | Raw | `xz -9` |
| :-- | --: | --: |
| `system-a`, whole partition | 671 088 640 | **765 156** |
| `kernel8.img` | 10 168 774 | 10 107 600 |
| `start4.elf` | 2 305 632 | 1 147 452 |
| `bcm2711-rpi-4-b.dtb` | 56 407 | 9 612 |
| `fixup4.dat` | 5 499 | 1 072 |
| `config.txt` / `tryboot.txt` | 931 each | 652 each |
| `cmdline.txt` ×2 | 136 each | 192 each |
| **Payload total** | | **≈ 12.0 MB** |

Three things follow, and together they settle it:

* **The half a delta would optimise is already under a megabyte.** The entire
  `/system` filesystem — five binaries, a directory tree, a generated `/etc` —
  compresses to **765 KiB**.
* **The half that dominates is incompressible and rarely changes.**
  `kernel8.img` and `start4.elf` are **94%** of the payload and change when
  Raspberry Pi ships firmware, not when Rivet ships a binary.
* **Per-member digests already give the useful part of a delta, for free.** The
  installer compares each boot-directory member's digest against what is on the
  target and skips the identical ones. A typical update therefore transfers
  ~12 MB but **writes 765 KiB and never opens a FAT filesystem for writing at
  all.**

Against that, a delta scheme costs:

* **Scratch space, and `ADR-018` §2 says where it lands.** All of a Rivet
  machine's card wear is concentrated in `/state`. Reconstructing a 640 MiB
  `/system` needs 640 MiB of it — on a card built at today's default that
  partition is **254 MiB total**, so it does not fit at all.
* **A fleet server serves the image it *booted*, never the one it downloaded.**
  It can serve a whole payload it retained; it cannot synthesise a delta for a
  version it has never held.
* **`ADR-016` §7 item 3's precedent, same project, same shape.** Layering was
  refused because *"layering is what you reach for when storage is scarce, and
  it is not."* Deltas are what you reach for when bandwidth is scarce, and on a
  LAN it is not.

### 6.1 Compressor

`gzip -9` 2 174 987 · `zstd -19` 784 650 · `xz -9` **765 156**. xz beats zstd by
**2.5%** on the only member that compresses at all — small enough that a
pure-Rust zstd decoder is nearly free, and `ADR-003`'s static-musl world has
reason to prefer one. **Open, `ADR-009` §3.2 requires the survey** (§12 item 2).

### 6.2 Firmware blobs are always carried — ruled 2026-08-23

**The instinct is that this is dangerous because A/B cannot protect the thing
that performs the rollback. That instinct is wrong here**, and checking it is
what settled the ruling:

* Each slot has its **own** boot partition with its own firmware, and a payload
  targets one slot. Break slot B's firmware, trial-boot it, it fails — and the
  rollback runs slot A's *good* firmware.
* The one thing that could genuinely brick the board — the bootloader EEPROM on
  the SoC — **is not on the card at all** and no payload can reach it.

So the real trade is self-sufficiency against 11 MB of transfer, and
self-sufficiency wins: **the payload fully defines a slot.** *"This artifact and
nothing else is what slot B becomes"* is a much stronger property than *"this
artifact plus whatever happened to be there"*, and it is the property that stops
a class of bug that only appears years later. Skip-if-identical means the device
writes nothing when firmware is unchanged, so the cost is bandwidth alone.

**This widens `ADR-018` §5.2's "its kernel", which was too narrow for a second
reason as well:** `cmdline.txt` is per-slot and *does* change between releases —
it gained `cgroup_enable=memory` on 2026-08-23. A payload carrying only a kernel
would strand that change on the slot the machine then boots. **The boot half is
a file set, not a kernel.**

---

## 7. Cryptography

### 7.1 Ed25519, raw — ruled 2026-08-23

**No X.509, no ASN.1, no DER.** `ADR-028` §8 item 1 left the scheme open and
warned specifically against inheriting it from `ADR-025` by proximity.

**What decided it is not the algorithm's strength — every candidate is fine —
but how much parsing happens before the signature can be checked.** X.509 and
ASN.1 are a long-running source of parser bugs, and §5 exists to keep the
pre-verification surface at five integer comparisons. A public key that is
**32 raw bytes** has no parser to get wrong. This is what OpenBSD's `signify`
and `minisign` do, for the same reason.

| Rejected | Why |
| :-- | :-- |
| RSA-4096 / PSS | 512-byte signatures, and padding is a real choice with a long history of being got wrong |
| ECDSA P-256 | Signing needs good randomness per signature; nonce reuse leaks the private key outright |

Ed25519 also needs no randomness at signing *or* verification, which matters on
a board with poor entropy early in boot.

**The warning in `ADR-028` §8 item 1 is respected rather than ignored.** It is
right about *keys*: the fleet identity key and the signing key are different
keys for different jobs, and `ADR-025` §2.2 already forbids coupling the device
identity to other key material. Ed25519 being the right answer twice is a property of the
algorithm, not a coupling. **Same algorithm, never the same key.**

The `sig_alg` field means adding a post-quantum scheme later is a field value
rather than a redesign.

### 7.2 BLAKE3, pure-Rust — ruled 2026-08-23, pending one measurement

`ADR-020` §6.2 establishes **on hardware** that the Pi 4's Cortex-A72 does not
implement the optional ARMv8 cryptography extensions (MIDR `0xd08`). That is why
§6.2 chose ChaCha20-Poly1305 over AES-GCM, and the same reasoning applies here:
**SHA-256 is the algorithm whose speed assumes hardware this board does not
have.** BLAKE3 is designed to be fast in software, which is the only kind of
fast available.

**Pure-Rust mode specifically** — the `blake3` crate ships C and assembly SIMD by
default, which would drag in the C cross-toolchain `ADR-001` §4.4 had to write a
rule about.

> **Corrected 2026-08-23, and the correction matters because the wrong version
> looks like it works.** `default-features = false` **does not give pure-Rust
> mode — it only drops `std`.** Read out of `blake3` 1.8.7's own `build.rs`,
> line 367: `|| (!is_no_neon() && !is_pure() && is_aarch64() && is_little_endian())`.
> Without the **`pure` feature**, a Rivet build compiles `c/blake3_neon.c`
> **for the board** and needs exactly the C cross-toolchain this paragraph exists
> to avoid. The dependency must be `features = ["pure"]`.
>
> This was found by an agent reading the build script rather than trusting the
> convention, after being handed the wrong instruction by me. `cmd/rivet`'s
> `Cargo.toml` quotes the line at the dependency so the next reader does not have
> to rediscover it.
>
> **A consequence for the measurement below:** on `aarch64-unknown-linux-musl`
> that build script emits no SIMD `cfg` at all and produces no object files, so
> **the board's BLAKE3 is the portable-Rust floor.** Whatever number the board
> gives is the worst case, not a tuned one.

> **One measurement settles three things**, and it has not been taken: this
> choice; whether reading a written slot back and hashing it is affordable
> (§9 step 9); and `ADR-015` §4.2's open cost, which already notes *"software
> hashing on a slow core, paid at every boot for every service."* Hash 640 MiB
> on the board with both algorithms.
>
> **Taking it needs a card reader**, and that is worth noticing rather than
> working around: there is no way to put a binary on a Rivet board today. No
> file copy, and no way to pipe bytes into a command, because
> `cmd/rivet` is the login shell and refuses shell metacharacters by design
>. The boot partition is the file-transfer channel
> (`ADR-002` §4.1). This is the transport question arriving from a
> direction nobody planned.

---

## 8. The `<kind>` token and the suffix

**`update`.** `AGENTS.md` §10.2 already fixes **Card image** and **Update
payload** as vocabulary, and `ADR-027` §4's table draws the distinction on
*provisions* versus *updates*. `card` and `update` are both words about what the
artifact **does**; `payload` is a word about what it is made of, and pairing it
with `card` would be incoherent.

**Suffix: `.rvt`, one suffix for all three artifact kinds, with `<kind>` saying
which.**

```
rivet-0.2.0-aarch64-rpi4-update.rvt
rivet-0.0.1-g7b84d4b-aarch64-rpi4-card.img.xz
```

The spelling is cosmetic. What is not cosmetic is that there is **one suffix and
the kind token distinguishes**, and that there is **no `.xz`** — which touches
two lines of `ADR-027`, recorded in §10.

---

## 9. How it fails safely

`ADR-018` §5's elegance is that the failure path needs no userspace. This format
does not add to that; it is designed to stay out of its way. The precise reason
a partially written inactive slot is not bootable:

> **Nothing in the payload path ever moves a selector.** There are exactly two —
> `[all] boot_partition=`, moved by `commit` after a healthy boot, and
> `[tryboot] boot_partition=` plus a one-shot reboot, moved by `arm_trial_boot`.
> Writing `p3`/`p4`, or files into a boot directory, moves neither. **The
> inactive slot is unreachable by construction** until arming, and arming is the
> last step, after every digest matched.

The property is not "the slot notices it is incomplete". It is **"the firmware is
never told to look at it"**.

| # | Failure | Why it is safe |
| :-- | :-- | :-- |
| F1 | Transfer truncated | The header names the last member's `offset + stored_len`; a short file is refused. Nothing written |
| F2 | Bad signature, unknown key, unimplemented algorithm | Refused at §5 step 7. **Nothing written** — which is most of `ADR-028` §8 item 4's answer: "what happens to a slot whose verification failed" largely dissolves, because verification precedes writing |
| F3 | A member digest mismatches after earlier members were written | The slot is partially written and **inert**. No selector moved. Report and leave it; the next update overwrites it. **Do not wipe it** — wiping is a second destructive operation on the one thing whose virtue is being unreachable |
| F4 | Power cut mid-write of `/system` | As F3 |
| F5 | Power cut mid-write of the boot half of slot A | **Not fully safe — see below** |
| F6 | Wrong board or architecture | `arch`/`platform` in the signed header; refused before any write |
| F7 | Correctly signed but older | **Reported, not refused** (`ADR-028` §6.1). The version is in the header, so enforcement later needs no flag day |
| F8 | System member larger than the partition | Refused before writing, from the GPT the machine can read |
| F9 | A member named `autoboot.txt` or `rivet.toml` | The **payload** is refused (§4) |
| F10 | Power cut between arm and reboot | On the Pi the one-shot lives in the reboot call, so the override is written but **inert**; the next ordinary boot is the default slot. On UEFI the arm persists — but by then the slot is complete and verified, so the trial is legitimate |

**F5 is the one that is not safe, and the record did not name it.** Writing the
boot half of slot A means writing into the unjournalled FAT filesystem that
holds the commit record. Three things bound it and none eliminate it:

1. **Skip-if-identical means the usual update never opens it.** This is the main
   defence, and it is a *safety* property rather than an optimisation.
2. **Order: system member first, boot half last.** The dangerous write is the
   shortest-lived.
3. **Per-file write-sync-rename**, the pattern `write_autoboot` already uses and
   which the record already trusts on FAT.

### 9.3 F5 is a confirmed hole — settled on hardware, 2026-08-23

This section was written asking what the firmware does with a **corrupt** — as
opposed to absent — `autoboot.txt`, and §12 item 3 filed it as needing a board.
**It got one the same evening. The answer is the unfavourable one.**

`p1`'s `autoboot.txt` was replaced with **1005 bytes of lorem ipsum**: present,
full-length, plain text, and containing **zero occurrences** of
`boot_partition`, `tryboot_a_b`, `[all]`, `[tryboot]` — or of the `=` character
at all. Verified by counting each on the card before the boot rather than by
eye. The board then booted **partition 1** (`root=PARTLABEL=system-a`,
`EXT4-fs (mmcblk0p3)`).

**That result is decisive, and it is worth saying why it is not the ambiguous
case it first looks like.** The card's `[all] boot_partition` had been `1`
before the file was damaged, so "booted p1" looks consistent with the firmware
merely having read a `1`. It is not: **there was no `1` in the file.** Nothing
parseable remained. Combined with the two boots on 2026-08-23
— where `p1` naming `boot_partition=2` produced a boot of `p2`, proving
the firmware reads and obeys the file — the pair is conclusive:

> **The firmware obeys `autoboot.txt` when it can, and falls back to partition 1
> when it cannot.**

**So the F5 scenario is concrete rather than theoretical.** Running slot B,
updating slot A, a torn write into `p1`'s FAT damages the directory and takes
`autoboot.txt` with it — and the next boot does not stop and does not stay on
B. **It boots partition 1, which is the half-written slot being installed.**

**None of §9's three bounds reach it.** Skip-if-identical helps only while the
firmware is unchanged; ordering the system member first does not help, because
the boot half is the dangerous write; and write-sync-rename protects
`autoboot.txt`'s *own* rewrite, not collateral damage from writing
`kernel8.img` beside it.

**RULED 2026-08-29 by the maintainer: the first of the three candidates below,
detect and repair. Built the same day; `PLAN.md` §94.2
carries how it was found still open and why that mattered.** The three are kept
below because the two that were not taken are the ones that would close the hole
rather than narrow it, and the second remains the real fix for a fleet.

Recorded so the decision is made deliberately rather than by whoever implements
first:

* **Detect and repair.** After writing a boot half, re-read `autoboot.txt` and
  rewrite it if it no longer parses. Cheap, and it *narrows* the window rather
  than closing it — a power cut can still land inside the gap.
* **Refuse the ordering.** Never write slot A's boot half while running slot B;
  require the operator to be running A first. Closes it, and makes updates
  order-dependent in a way that will surprise somebody.
* **Move the selector out of the fallback's reach.** Ideal, and probably
  impossible on this hardware: `p1`'s `autoboot.txt` is the only thing the
  firmware reads.

### 9.3.1 What was ruled, and what is still true — 2026-08-29

**Detect and repair is built** (`cmd/rivet-deploy/src/install.rs`). The installer
reads `autoboot.txt` on the read-only mount, before the boot half is opened for
writing, and checks it again after the writes are synced; a difference is
restored from the bytes read first, by write-sync-rename, and reported at `warn`.

**It compares bytes rather than re-parsing, and that is deliberate.** The
installer is never entitled to change this file — §6 makes the name reserved and
§9.2 step 5 refuses a whole payload carrying one, while writing the selector is
*arming*, a separate operation under §9.1 — so **any** difference is damage by
definition. Byte equality is therefore both simpler and **strictly stronger**
than parsing: a parse check passes a `boot_partition=2` that became
`boot_partition=1` under one flipped byte, which is a card that silently boots
the wrong slot, which is the failure this section is about.

**The warning in §9.2's install record stays, because the risk is narrowed and
not removed:**

> **Power lost between the damaging write and the repair leaves the selector
> damaged**, and the next boot is then the one this section describes.

So the sentence this replaces is **weakened rather than withdrawn**: `ADR-018`
§5.2's flow targeting slot A from a machine running slot B is no longer
untrusted, but it is not equivalent to the other direction either. **Closing the
gap is the second candidate — refusing the ordering — and it is still unruled.**
It is the one that matters for a fleet rather than a development board, because
the development board is `ADR-035` §8A.3's blast radius and a fleet machine is
not.

**Not proven on hardware.** No torn write has been induced on a real card; three
tests exercise the detection and repair over a scripted kernel.

### 9.3.2 RULED 2026-08-29: the cross-slot direction is refused, and the register may permit it

**The maintainer ruled §9.3.1's open candidate, and ruled it as both of the two
options put to him: refuse the ordering, AND make the refusal a policy the
register carries.** *(`.rulings/` R-104.)*

> **Installing into the slot that is not running, from a machine running the
> other, is REFUSED by default.** The operator restarts into the target slot
> first. A device record in the fleet register may permit it explicitly, and a
> machine with no register entry gets the refusal.

#### Why both, rather than one of them

They answer different halves. **Refusing closes the hole**; nothing else does —
detect-and-repair narrows a window and a narrowed window is still a window, and
`ADR-018` §5.2's own text concedes the two directions are not equivalent. **The
policy is what keeps the refusal from being a lie about the fleet Rivet actually
has**: a development board on a desk is `ADR-035` §8A.3's blast radius — an SD
card write — and refusing there costs a workflow for a risk the operator has
already accepted by sitting next to the machine.

A single unconditional refusal would have been simpler and would have refused
the deploy this project performed on the day it was ruled. That is not an
argument against the refusal; it is the argument for the exemption being
*recorded per machine* rather than being a flag somebody types.

#### The default is REFUSE, and that is the part with a cost

**A machine the register does not know about is refused.** The safe answer has
to be the default, because the case this protects is the machine nobody can walk
to, and that machine is exactly the one most likely to be missing a considered
policy entry.

**This changes the development workflow, and saying so is not a caveat.** Until
the test board's register entry carries the permission, a cross-slot install is
refused and the operator restarts into the target slot first. The permission is
one field on one device record; the point of putting it there rather than in a
command-line flag is that **a flag is typed by whoever is in a hurry, and a
register field is a decision somebody made once and can be reviewed as a diff**
(`ADR-034` §3.4).

#### What the refusal must say

`ADR-012` §7.4's doctrine applies and this is a good test of it. The refusal
names the running slot, the target slot, the exact reason — a torn write into the
shared FAT partition can damage the boot selector, and power lost between the
damage and the repair leaves it damaged — **and both ways forward**: restart into
the target slot, or record the permission on this device. A refusal that named
only the first would push an operator toward the reboot every time, including on
the fleet machine where the reboot is the expensive answer.

#### What this does not settle

* **Whether the permission is one boolean or a richer policy.** One boolean is
  what today needs. `ADR-034` §3 makes the register a store of public facts and
  a policy field is exactly that, so growing it later is additive.
* **How a machine with no register entry is reached at all**, which is the
  enrollment question `ADR-019` owns and is unaffected by this.
* **It is not built.** No code refuses anything yet, and the register has no such
  field. Recorded here so the ruling is not the thing that goes missing —
  `PLAN.md` §94.7 is the whole reason this section exists in this shape.

### 9.1 Install and arm are one operation; the reboot is separate — ruled 2026-08-23

The question raised was whether install and arm must be separable, so a fleet
could be updated during the day and rebooted at 2am — which would need
persistent "this slot is written and armable" state in `/state`, and that
marker's authority would then need ruling.

**It does not, and the reason is already built.** `arm_trial_boot` does not
reboot. On the Pi it writes the `[tryboot]` override while the one-shot itself
lives in the *reboot call* (`ArmedReboot::OneShot`), so **an armed-but-not-
rebooted machine is completely inert**: `[tryboot]` applies only during a tryboot
boot, and an ordinary reboot in the meantime boots the current default,
harmlessly.

So install-now-reboot-later already works, **with no marker and no new
authority**. Twenty machines can be installed and armed at noon and rebooted at
2am. The rejected alternative — three operations and a `/state` marker — buys
install-now-*arm*-later, which nothing needs, and costs a marker that can
disagree with reality.

### 9.2 The install sequence

1. Read and verify the header (§5 steps 1–7).
2. Refuse on `product` / `arch` / `platform` / `kind` mismatch.
3. Compare `version` to the running one; **report** if not newer; never refuse
   (`ADR-028` §6.1).
4. `running_slot()` → `other_slot()` → `payload(target)` from `BootSlots`.
5. Refuse if the system member does not fit, or if any member name is reserved.
6. Per member: read `stored_len` at `offset`, digest, compare. Members are
   765 KiB / 10 MB / smaller, so this needs no staging file and therefore adds
   **zero `/state` wear** — `ADR-018` §2 honoured rather than worked around.
7. Write the **system** member (whole partition), `fsync`.
8. Write the **boot-directory** members, skipping any whose digest already
   matches the target, via write-sync-rename.
9. **Recommended:** read the written system partition back and hash it. SD cards
   do accept writes and store something else, and this is the only defence
   against that. Cost is one 640 MiB read and hash — §7.2's measurement.
10. `arm_trial_boot(target)` → `ArmedReboot`; the caller reboots accordingly.
11. The new slot boots; PID 1 confirms. Unchanged.

**Steps 1–10 are `cmd/rivet-deploy`'s since 2026-08-24 — §14.** Step 10's
*"the caller reboots accordingly"* is the one clause that did **not** become
that binary's: §9.1 rules the reboot separate, and an armed machine is inert
until one, so the installer stops after the arming and reports which kind of
reboot carries it.

---

## 10. What this changes in things already ruled

Named rather than worked around.

| Where | Change |
| :-- | :-- |
| **`ADR-018` §5.2, step order** | **Direct contradiction, corrected.** The flow read *"1. Write the new image … 2. Verify the signature"* — it wrote attacker-controlled bytes to a partition before establishing they were ours. **The steps are inverted**, and that inversion is what makes F2 and F6 write nothing at all |
| **`ADR-018` §5.2, step 1's contents** | *"its kernel"* is too narrow. The boot half is a **file set** — §6.2 |
| **`ADR-027` §3, the grammar line** | Hard-codes `.img.xz`. §9 item 2 already conceded the suffix belongs to whoever rules the format; the pattern becomes `<product>-<version>-<arch>-<platform>-<kind><suffix>` |
| **`ADR-027` §3.4** | Says compression is part of the name. The payload has **no uncompressed variant that moves**, so a suffix naming compression would name a property with no alternative — which §3 itself calls a sign the grammar is wrong. Recorded as a per-kind consequence rather than left for someone to "fix" |
| **`ADR-027` §9 item 1** | **Answered: `update`** |
| **`ADR-015` §4.4** | Its row says *"signature before `tryboot`"*, which permits write→verify→tryboot. **Tightened to before write** |
| **`ADR-028` §8 item 1** | **Ruled: Ed25519, raw** — §7.1 |
| **`ADR-028` §8 item 4** | Partly answered: nothing is written when verification fails, and a slot left partial is **left inert, not wiped** |
| **`rivet-adapter-image`'s `BootSlots`** | Gains one operation: the names a platform reserves on a payload target (§4). **Done 2026-08-24** — `reserved_names(&PayloadTarget) -> &'static [&'static str]`; §13. **And a second, later the same day — `platform_token()`**, `ADR-027` §3's `<platform>`, which §9.2 step 2 compares against a signed header; §14.3 |
| **`rivet-adapter-image`'s modules** | Gains `install` — §9.2's steps 4, 5, 7 and 8 as a **plan**, with no I/O. **Done 2026-08-24, §14.2.** The crate still has one dependency and still forbids `unsafe` |
| **`rivet-adapter-image`'s `Cargo.toml`** | It has no dependencies and `unsafe_code = "forbid"`; a verifier needs crypto. **The header layout, ordering and refusals are pure computation and belong in `rivet-core`** — which has an empty dependency table by design — behind a `Verifier`/`Digest` port, with the crypto crate landing at the composition root exactly as `Kernel` and `BootMedium` already do. This also makes the property that matters — *nothing is written before verification* — testable against a fake, which a hard crypto dependency would not |
| **`cmd/cargo-rivet`** | **The payload emitter is `cargo rivet update` — ruled 2026-08-24, §15.** This row named `cmd/rivet-image` until then; the emitter lives here because `cmd/cargo-rivet` is the only crate in the workspace that holds a **signing key**, and it did not exist when the row was written. The half of the row that survives is the observation about erofs: `disk::mkfs_erofs_args` already builds erofs to a standalone file, so once §3.1's ext4 stopgap retires the system member is produced by the existing path, and the ext4 case builds in place at an offset and needs extraction. That is now about this verb's `--system` input rather than about where the code lives |

---

## 11. One container serves all three artifact kinds

`ADR-015` §4 says *"one signing, verification and transport path serves all
three"* — **path is not format**, and those are different claims. Examining them:

* an **application** is one static binary, a typed manifest, optional assets, a
  signature (`ADR-015` §3);
* an **extension** is one signed, versioned, read-only image (`ADR-016` §2);
* an **update payload** is N members across two target classes.

All three are *a signed set of named members plus typed metadata*. What differs
is the **interpretation** of the metadata, not its shape.

**The decisive argument is `ADR-028` §2's last row again: the verifier is the
most security-sensitive parser on the machine. Three formats means three
verifiers, each with a third of the scrutiny.** One is strictly better.

This does **not** rule the application and extension cases — `ADR-027` §9 items
2–4 are theirs. What is ruled is that the payload is a *case* of a general
container and must be built so the other two can be cases of it too. What would
falsify that: an application manifest whose grammar cannot fit a fixed record
table. `ADR-015` §3.2's manifest is key/value with enumerated values, so it can.

### 11.1 Tested twice, and the second time by a kind this section did not
anticipate

**The heading says "all three" and there are now four.** `SPEC-02-Shared-Rivets.md`
added `library` on 2026-08-31 — a kind nothing in this ADR imagined, arriving on
exactly the terms `application` did in 2026-08-24:

| | changed to add `library`? |
| :-- | :-- |
| `package::read` — the verifier | **no** |
| `package::write` — the builder | **no** |
| `package::format` — §3's layout | **no** |
| `package::kind` — the token's validation | **no** |
| a new module holding one interpretation | yes, `package::library` |
| one branch in the installer, below the shared verification | yes |

That is this section's claim being **true rather than aspirational**, and the
evidence is stronger for the kind being unanticipated: a design that had been
shaped around three known cases would not prove much by accommodating a fourth
it was shaped around.

**A fifth kind, `apk`, is designed on the same terms** (`ADR-061` §3): an
Android APK as a stored member beside its metadata, signed by the application
key, held by a fleet server for a phone to fetch, and refused by a machine's
installer. It is a token, one module of interpretation and one installer branch
that refuses, and it asks nothing of the verifier, the builder or the layout.

**The heading is left as it was written** rather than corrected to "four", per
`AGENTS.md` §2 rule 3: the number was right when it named what the record then
contained, and the sentence under it — *one container, one verifier, per-kind
interpretation* — is the part that governs and it did not change.

---

## 12. Open

1. **Whether the three kinds share a trust set** — `ADR-028` §8 item 3. A shared
   container makes the question sharper, not softer.

   **Answered for two of the three, 2026-08-24 — `ADR-028` §5.5.** They share a
   trust *store* and not a trust *set*: one file lists the keys, and each entry
   carries the kinds that key may sign. So a key held for applications cannot
   ship an operating system, and the check happens after the signature verifies —
   which is the point, because *"the signature being genuine is what makes 'and
   yet it may not sign this' worth saying"*. `extension` has no consumer yet.
2. **Decompressor crate** — `ADR-009` §3.2 requires the survey. §6.1 narrows it:
   xz's 2.5% edge over zstd is small, and the kernel should be stored
   uncompressed anyway.

   **CLOSED, two ways — 2026-08-29 and 2026-09-13.** For an update payload:
   none, ever (§13, R-107 — a decompressor is a permanent device dependency
   this format does not need). For a `card` image alone: zstd, on the
   workstation only (§13.5, R-177.1) — `cmd/cargo-rivet`'s own dependency,
   never `rivet-core`'s, so no board binary gains one either way.
3. **What the Pi firmware does with a *corrupt* `autoboot.txt` — CLOSED on
   hardware 2026-08-23, §9.3: it falls back to partition 1, so F5 is a confirmed
   hole.** The follow-on question, which of §9.3's three mitigations to take,
   was **RULED and BUILT 2026-08-29 —
   §9.3.1: detect and repair.** What remains open is narrower again, and is the
   only one of the three that would *close* the hole rather than narrow it:
   **whether to refuse writing slot A's boot half from a machine running slot
   B.** It matters for a fleet and not for the development board, and it is
   deliberately left rather than forgotten — `PLAN.md` §94.7.
4. **Which process installs an update — RULED by the maintainer 2026-08-24,
   §14. A new supervised binary, `cmd/rivet-deploy`.** The consequential
   half is answered with it: **`rivet-adapter-image` ships no `BootMedium`
   implementor**, and the new binary supplies one at its own composition root.
   What that ruling did **not** settle is §14.4's two questions, which are open
   and are the maintainer's.

   **And it installs applications too, 2026-08-24 — `ADR-015` §7.1.** The same
   binary, dispatching on the kind after verification, on §11's argument applied
   one level up: everything before the dispatch is the security boundary and a
   second copy of it is the one that diverges. **The consequence is that the
   binary's name is now half wrong**, and renaming it is `PLAN.md` §0.4.6 rather
   than something this build decided.
5. **Payloads cannot express a layout change.** The format deliberately cannot
   say "grow the `/system` slots". If a future release needs a different
   partition table, the answer is a card image and a reflash. Stated so nobody
   discovers it.
6. **Whether per-member skipping is right for firmware blobs specifically.** It
   assumes the firmware is content-addressable by filename, which it is on this
   platform and which has not been proven across firmware releases.
7. **How two version strings are ordered** — added 2026-08-24, §13.4. §6.1 of
   `ADR-028` requires a non-newer payload to be *reported*, which needs an
   ordering, and nothing in the record says what `0.2.0` versus
   `0.0.1-g7b84d4b` means. The implementation carries the string and refuses to
   compare, because a wrong ordering would report a downgrade as an upgrade —
   `AGENTS.md` §5's *"a wrong guess does not fail, it succeeds"*.

---

## 13. What was built — 2026-08-24

`AGENTS.md` §6 and §7. This section is a dated observation about a tree, not a
claim that survives being read later; count `Cargo.toml`'s members and run the
suite rather than believing a number here.

### 13.1 Where it lives, and why there

| What | Where | Why |
| :-- | :-- | :-- |
| the format: reader, writer, verifier, refusals, `<kind>`, `update`'s interpretation | `rivet-core/src/package/` | §10's own row: *"the header layout, ordering and refusals are pure computation and belong in `rivet-core`"*. Its `[dependencies]` table is **still empty** |
| `Digest`, `Verify`, `Sign`, `TrustSet` | `rivet-core/src/package/port.rs` | §10: *"behind a `Verifier`/`Digest` port, with the crypto crate landing at the composition root"* |
| BLAKE3 and Ed25519 | **`rivet-crypto`**, a new crate | below |
| `BootSlots::reserved_names` | `rivet-adapter-image` | §4: *"this is the port's job, not the installer's"* |

**`rivet-crypto` is a new crate and `ADR-014` §2 does not list it.** §5 of that
ADR rules that *"a crate is created by the first thing that needs it"*, and the
thing that needed it is `ADR-014` §2.2's argument arriving a second time: the two
callers are `cmd/rivet` (verifies on the board) and `cmd/rivet-image` (signs
on the host), **both binary crates with no lib target**, so putting the
implementors in either would leave the other unable to name them and the verifier
— the most security-sensitive code in the project — existing twice.

It is **not** called an adapter, deliberately. An adapter hides a platform; this
hides a dependency. It sits beside `rivet-config` in the layout: no I/O, no
syscall, and no type of its own on `rivet-core`'s surface (`ADR-009` §4).

**The ports in `rivet-core/src/package/port.rs` are the first *domain* ports
this repository has** — `AGENTS.md` §3.1's table says *"none yet"* against that
row, and §3.1's own test is whether the signatures name anything the core is
forbidden to name. They do not; everything moves `&[u8]` and fixed arrays. What
the core cannot do is not *reach* the world but *link an implementation*.

### 13.2 What the implementation had to decide, because this ADR did not

Each is small, each is recorded at its definition, and each is the maintainer's
to overturn.

1. **Little-endian**, for every integer field. §3 gives widths and offsets and
   never names a byte order. Both targeted architectures are little-endian.
2. **Reserved bytes must be zero**, including the member records above
   `member_count` and the 24 bytes between the table and the signature. Refused
   otherwise. This costs nothing, because §5 step 3 already refuses any
   `format_version` but `1`, so a new field costs a version bump regardless —
   and it buys there being no place in a signed header for bytes no reader looks
   at.
3. **Members are contiguous from byte 4096, and the file ends where the last one
   does.** A gap or a trailing tail is refused, so no byte of the file is outside
   every digest.
4. **A member's identity is `(name, variant)`, not `name`.** §3 requires
   `cmdline.txt` to appear twice, so name alone cannot be the key. The byte §3
   calls `slot_variance` is exposed generically as `variant`, because §11 makes
   the container general and *slot* is the update kind's word; it is the same
   byte at the same offset.
5. **`platform` may be empty.** That is how `ADR-027` §9 items 3–4 stay
   decidable: an application may turn out not to carry one, and a format that
   required it would have answered the question.
6. **`Kind` is a validated token, not an enum**, for the same reason. It has one
   constant — `update`, which §8 ruled.
7. **`verify_strict`, not `verify`.** Ed25519's small-order keys and
   non-canonical encodings are the documented interop hazard, and the verifier
   is the last place a project should discover two libraries disagree about
   validity. `ed25519-dalek` was taken over `ed25519-compact` largely because it
   exposes the strict form; `ring` was refused outright for its assembly
   (`ADR-001` §4.4). `cargo deny check licenses` needed **no** addition, and an
   `aarch64-unknown-linux-musl` build of the whole closure emits no object files.
8. **A signature is 64 bytes and the format has room for exactly that.** §7.1
   says a post-quantum scheme is *"a field value rather than a redesign"*, which
   is true of the identifier and not of the size: a larger signature needs
   `format_version` 2. Stated so nobody discovers it.

### 13.3 A consequence found rather than designed

**Ed25519 is defined over SHA-512, so `sha2` is now permanent in any image that
verifies a payload.** `cmd/rivet`'s `Cargo.toml` plans to delete that
dependency once §7.2's measurement is taken, *"because nothing else in Rivet
uses SHA-256 and nothing is planned to"*. Half of that stays true — nothing uses
SHA-**256** — and *"the image carries no SHA-2 implementation"* stops being true
the moment anything verifies. The measurement's own deletion is unaffected.

### 13.4 What is **not** built, and each is deliberate

* **Compression.** §12 item 2 has not ruled a decompressor, and §5.1 puts
  compression in the one position the design was shaped to protect. `compression`
  is read and any value but `0` is refused.
* **Version ordering.** §12 item 7, added above.
* **The installer — built later the same day, §14**, and this list said nothing
  wrote to a slot in shipped code for the few hours before it landed. The
  maintainer ruled §12 item 4 and `cmd/rivet-deploy` was built against it.

  **The half of that bullet that was a prediction turned out right and is worth
  keeping:**
  what §13 built was *"everything an installer needs whichever process owns it,
  including a `VerifiedMember` token a caller cannot construct: an installer
  that takes one cannot be handed unverified bytes by mistake."* §14.3 is that
  token doing the job it was made for.
* **Key management.** `ADR-015` §4.3 makes the maintainer the gatekeeper and
  there is no key story anywhere. `TrustSet` holds keys and has no opinion about
  where they came from — including **no opinion on §12 item 1**, whether the
  three kinds share one.
* **§9.3's F5 mitigation.** Still unruled, and it is a ruling rather than an
  experiment. `ADR-018` §5.2's flow should still not be trusted for an update
  that targets slot A from a machine running slot B.
* **A payload emitter, and a CLI verb in `cmd/rivet`. Half of it arrived
  2026-08-24 — §15**, and neither existed when this list was written.
  `cargo rivet update` is the
  emitter, and it is in `cmd/cargo-rivet` rather than `cmd/rivet-image` for
  the reason §15.1 gives. **A CLI verb in `cmd/rivet` is still absent** and is
  a different question, since the maintainer's 2026-08-24 ruling gave the install to
  `cmd/rivet-deploy`.

### 13.5 What has been run, and on what

* The suite, under WSL. Bytes the writer did **not** produce are most of it —
  truncations, single-bit flips in the magic, the manifest and the signature,
  reordered member records, a record claiming a length the file does not have,
  duplicate name-and-variant pairs, a path traversal, a forged `key_id` naming a
  trusted key, an empty archive. **RFC 8032 §7.1's own vectors** and BLAKE3's
  published empty-input digest are in there, and they are worth more than
  anything composed here (`AGENTS.md` §4).
* **Eight defect injections**, each run and each watched to fail the tests named
  for it. One of them found a test that could not fail: the `verify_strict`
  assertion passed with strictness removed, and was replaced with a vector that
  tells the two apart.
* **A loop device**, not a card. `rivet-crypto/tests/loopback.rs` builds a real
  payload from a card image's `system-a`, verifies every member, writes 640 MiB
  to the inactive slot's partition, `fsync`s, and reads it back and hashes it —
  then offers three corrupted payloads and asserts the slot is byte-identical
  afterwards. It also re-measured §3's claim against the card in use: of nine
  files on both boot partitions, **`cmdline.txt` is still the only one that
  differs.**

**`ADR-026` §4 puts `/dev/mmcblk0` in the column QEMU cannot reach, and a loop
device is not a card.** It cannot exercise the SD controller, a card that accepts
a write and stores something else — which is §9.2 step 9's whole reason for
existing — a card gone read-only mid-write, or erase-block behaviour. **Nothing
in this section has run on a Raspberry Pi**, and §7.2's measurement is still not
taken.

---

## 14. The installer — ruled 2026-08-24, and built the same day

§12 item 4 asked *which process installs an update*. This section carries the
answer, what was built against it, and the two questions it deliberately does
**not** answer.

> **CORRECTED 2026-08-29 — the binary is `cmd/rivet-deploy`, and this document
> called it ~~`cmd/rivet-update`~~ fourteen times.** The maintainer renamed it on
> 2026-08-24, the same day it was built, while changing the name still cost
> nothing *(`.rulings/` R-278)* — the
> binary installs system updates **and** applications, so `update` described half
> of it. The rename went through the tree and never reached the record; the old
> name survived here and in `ADR-015` §7.1, `ADR-018` §5, `ADR-027` and
> `ADR-028`, twenty-two sites in all, until `tools/docs/claims.py` was pointed at
> them. Every occurrence in those five documents is now `cmd/rivet-deploy`,
> corrected rather than struck because a **name is a pointer and not a ruling**:
> striking twenty-two of them would bury the sentences they are in, and a reader
> following the old one reaches an empty path. The rename itself is the record
> — `ADR-033` §6 lists the binaries — and it stands unaltered. **`AGENTS.md` §7
> is the rule this broke**, in the direction nobody watches: not a stale claim of
> absence but a stale claim of *presence*, five days old and confidently spelled.

### 14.1 The ruling

**Ruled by the maintainer, 2026-08-24:** an `.rvt` is installed by **a
new supervised binary**, `cmd/rivet-deploy` — not by `cmd/rivet`, which was
the recommendation.

The argument that decided it was the caveat rather than the case:

> **An update writes 640 MiB, and doing that from the login shell means a remote
> session owns a long write.** A connection that drops mid-write owns a
> half-written slot.

**The consequential half of item 4 is answered with it, in the direction the
port had already anticipated.** `rivet-adapter-image`'s `port` module said:

> **That is a decision with a known expiry.** `ADR-028` §4 says an authenticated
> principal supplies the authority to install an update and does not say which
> process executes it. If that turns out to be anything other than PID 1, this
> is where the question of a second implementor is answered.

**Answered: no.** `rivet-adapter-image` ships no `BootMedium` implementor and
keeps its empty dependency table and its `unsafe_code = "forbid"`. A
`StdBootMedium` of its own would need `libc` for `mount(2)` and would stand a
**fourth** syscall authority beside the three that already exist
(`cmd/rivet-initd`, `cmd/rivet`, `cmd/rivet-timed`). `cmd/rivet-deploy`
supplies one over its own seam, exactly as those three do.

### 14.2 Where the parts live, and the one placement that is not obvious

| What | Where | Why |
| :-- | :-- | :-- |
| the container, the verifier, `update`'s interpretation | `rivet-core/src/package/` | unchanged — §13.1 |
| BLAKE3, Ed25519 | `rivet-crypto` | unchanged. **Its manifest named two callers and there are now three**, which is that crate's argument arriving a second time |
| **the install plan** — which member to which target, in what order, which are skipped | **`rivet-adapter-image/src/install.rs`** | below |
| performing it — every byte of I/O, the journal, the exit codes | `cmd/rivet-deploy` | it is the composition root |

**The plan is not in `rivet-core`, and the instinct that it should be is worth
answering rather than leaving.** `AGENTS.md` §3 puts pure domain logic in the
core, and "which member goes where, in what order" is pure. But **its vocabulary
is the adapter's**: a step names a `PayloadTarget`, a device node and a
`SlotId`, and `ADR-014` §3's table forbids core to adapter *precisely so that
naming one of those from `rivet-core` fails to compile rather than being
caught in review*. It is the same argument `rivet_core::package::update`
already records against its own `TargetClass`, one level out.

What stays in the core is everything sayable without them, and it is most of the
refusals: `Payload::open` on reserved names and slot-variance completeness,
`Payload::accepts` on product/arch/platform, and `Package::verify_member` as the
only route to a `VerifiedMember`.

### 14.3 Two properties are structural rather than commented

1. **Nothing is written before every member verifies.** The write takes a
   `VerifiedMember`, which a caller cannot construct, so an installer that wrote
   first would not compile. §13.4 anticipated this token's purpose exactly.
2. **The system member is written before the boot half** — §9's second bound on
   F5. `InstallPlan` holds the two in separate fields, so there is no list a
   caller could iterate the wrong way round.

**Two more are behaviour rather than type, and each has a test that fails
without it:** the boot half is not opened for writing when every file already
matches (§6's skip, which §9 makes a *safety* property), and the arming is last.

`BootSlots` gains one operation, `platform_token` — `ADR-027` §3's
`<platform>`, which §9.2 step 2 compares against a signed header. It is
deliberately not a shorter `platform()`: §3.1's table makes them different
questions, and a reword of a console line must not refuse a payload.

### 14.4 What the ruling did NOT settle, and neither does this section

**Both are the maintainer's** (`AGENTS.md` §1). The maintainer named them in
the same paragraph as the 2026-08-24 ruling: *"whether it is one-shot or long-running, and
how an operator asks it to install something."*

The implementation takes the **smallest thing that works** and says so at the
code rather than burying it — `cmd/rivet-deploy/src/args.rs`'s module
documentation is the record, and the maintainer's 2026-08-24 install-request ruling carries the alternatives:

* **One-shot**, on `cmd/rivet-netd`'s precedent — `ADR-017` §4.3 ruled that
  *"run-once needs no new vocabulary"*, and `on-failure` plus the supervisor's
  `Done` state is exactly this shape.
* **The payload's path on `argv`.** Not `ADR-017` §3.4's notify socket, which is
  `sd_notify`'s convention and runs *service to supervisor* — the wrong
  direction for an instruction.

**What that leaves is the thing to flag rather than hide.** A system service's
argv is `cmd/rivet-sysd`'s `definitions::system_services()`, a
compile-time constant carrying no parameter — and
running the binary from the login shell is the hazard ruling 1 exists to remove.
The binary is the half that can be built and tested now; the channel is a
ruling.

**The channel was ruled on 2026-08-24 evening**, and until then there was no way
for an operator to start this binary with a path through the supervisor at all
— see §14.7.2. `definitions::deploy(payload)` takes the parameter
`system_services()` had nowhere to get, and a request file is where the payload
comes from.

### 14.5 A third thing this could not decide, and did not

**`ADR-028` §5.2's root key set has no location.** It rules that the set ships
in `/system`, read-only by format; §13.4 records that there is no key story
anywhere. So `cmd/rivet-deploy` takes `--trusted-key PATH`, repeatable and
**required**, and refuses with an explanation when given none. Inventing a path
under `/system` would be an installer claiming authority over where a machine's
trust anchors live (`ADR-011`).

Likewise **the actor**. `ADR-028` §4 makes authority an authenticated
principal's and `ADR-012` §4 requires provenance; nothing carries a principal
into this process, so every record's `actor` is `update` — this process acting
on the instruction it was given. A record naming a person would be inventing
one.

### 14.6 What has been run, and on what — `AGENTS.md` §6

**A dated observation about a tree, not a claim that survives being read
later.**

**The shipped binary, against a loop device over a copy of a real 2 GiB card
image, under WSL as root.** Not the emulator: `ADR-026` §4 puts `/dev/mmcblk0`
in the column QEMU cannot reach at all, so a guest has no slots to write.

The machine was faked in exactly two places and nowhere else — a bind-mounted
`/proc/device-tree/chosen/bootloader` inside a mount namespace, because x86 has
no device tree, and `/dev/mmcblk0p1` to `p4` as nodes carrying the loop
partitions' own major:minor. Everything else was real: a real signed
683 631 182-byte `.rvt` built from that card's `system-a`, both `cmdline.txt`s,
the firmware blobs and the kernel; a real 640 MiB write; a real `fsync`; a real
`vfat` mount; a real `autoboot.txt` rewrite.

What ran, in both directions:

| Run | Outcome |
| :-- | :-- |
| a dry run | verified, planned, wrote nothing, and checked the arming — `ADR-006` §6.2 |
| one flipped bit in the system member | refused at §5 step 8, **no device write at all** |
| one flipped bit in the signed header | refused at §5 step 7, before the manifest was parsed |
| **running slot A, updating slot B** | 640 MiB written, read back, digest matched, boot half **skipped entirely** (7 of 7 identical), `[tryboot]` armed. Exit 0 |
| a boot file made to differ | that one file written by write-sync-rename, six skipped, the file byte-identical afterwards, **no `.new` left behind** |
| **running slot B, updating slot A** | §9.3's F5 warning emitted **at the moment it applied**, `[tryboot]` moved 2 to 1, `[all]` untouched — and **`autoboot.txt` and `rivet.toml` byte-identical afterwards**, which is §4's whole reason for existing, watched rather than argued |
| a payload whose `arch` is not this binary's | refused before any write — F6 |
| no `--trusted-key` | refused, naming §5.2 |

**It found one defect, and that is the most valuable thing in the run.** The
installer's `mount` treated *every* `EBUSY` as "already mounted here". It is
not: the kernel also answers `EBUSY` for a device already mounted elsewhere with
an incompatible flag, which is what a machine running slot A does to `p1` while
PID 1's provisioning mount is in place. The read path then failed honestly with
`ENOENT` — but the **write** path does not read first, so over a mount that
never happened it would have put the payload's kernel into a directory on the
root filesystem and reported an install. Tightened to compare `st_dev` against
the parent's, and the case is now run on purpose.
`cmd/rivet/src/boot.rs`'s `SyscallMedium` carries the untightened form.

**What none of this settles**, and the list is the important half:

* **Nothing has run on a Raspberry Pi.** A loop device cannot exercise the SD
  controller, a card that accepts a write and stores something else (§9.2 step
  9's whole reason for existing — the *only* place that hazard is produced is a
  unit test's fake), a card gone read-only mid-write, or erase-block behaviour.
* **No trial boot has been performed from an install.** The arming was written
  and read back; nothing rebooted. `ADR-018` §5.3.2 settled the arming and the
  rollback on a board *by hand*, and this binary has not been the thing that
  armed one.
* **The binary was x86-64.** It cross-compiles clean to
  `aarch64-unknown-linux-musl` — a static ELF — and **that has never executed**.
  The bench payload therefore carried `arch = x86_64`, named in the fixture
  rather than defaulted so a reader can tell which run was which; F6 refused the
  `aarch64` payload correctly, and that refusal is one of the runs above.
* **The timings are not §7.2's measurement and must not be read as one.**
  Release build, x86-64 with AVX2, a loop device over WSL's own filesystem with
  the page cache warm: verifying nine members about 0.19 s, writing and syncing
  640 MiB about 0.43 s, reading it back and hashing about 0.27 s, the whole
  install under a second. **A board differs in all three axes.** What the
  *shape* says is unchanged from §13.5's loop-device run: the read-back costs
  about what the verification costs.
* **§9.3's F5 mitigation is still unruled.** The installer **reports** it and
  does not choose, because refusing would be taking §9.3's second option by
  implementation.

---

### 14.7 What 2026-08-24 evening closed, and what it narrowed

§14.4 named two things the ruling did not settle: **whether the installer is
one-shot or long-running**, and **how an operator asks it to install
something**. The maintainer ruled the first and left the second.

#### 14.7.1 Closed: one-shot, on the card, with a definition

| | |
| :-- | :-- |
| **one-shot** | ruled — `ADR-017` §4.5. `cmd/rivet-sysd`'s `definitions::deploy(payload)`, restart policy `never`; §4.5.2 argues why it is not `netd`'s `on-failure` |
| **on the card** | `cmd/rivet-image` installs the binary at `rivet_core::deploy::PROGRAM_PATH`, an eighth required positional. §14.4's *"a card that carries the program"* precondition is met |
| **the operator's surface** | `rivet apply install <digest> confirm` and `rivet get install` — `ADR-006` §8.8 |

**One thing arrived that was not asked for and is worth recording**:
`rivet_core::deploy` now owns the installer's name, its path, its actor and
the six-value `outcome` token. It is the shape a shared vocabulary module takes, and it exists
because four crates — three of them binaries with no library target — acquired
an opinion about the same words at once. The same standing debt has been
tracked for six other `/system/bin` paths since 2026-08-23; this is the
first one paid, and it is paid only because the module had to exist anyway.

#### 14.7.2 The channel — RULED 2026-08-24 evening: option A — and it was narrower than §14.4 left it

**Ruled the same evening this section was opened.** The maintainer took
the maintainer's 2026-08-24 ruling:
`rivet apply install <digest> confirm` writes a request naming the payload onto
`/state`, and the supervisor takes it up on its next convergence pass. The
amendment it needed — a service instance created at run time — is `ADR-017`
§3.2.2, and `ADR-017` §6.1 records that `enabled` was left alone. The section
below is kept whole because it is what the ruling was taken on.

**What the request names became a DIGEST on 2026-09-12** (`ADR-052` §7.2): the
supervisor resolves it in the machine's own artifact store and hands
`cmd/rivet-deploy` the path it found. The installer's contract is untouched — it
still takes a path on its argument vector, still opens it, and is still the one
verifier §11 allows. What changed is that the path is now derived on the
supervisor's side rather than supplied from the operator's.

**§14.4 said there is no way to start the binary with a path through the
supervisor, and there is now.** Two facts were checked on 2026-08-24 rather than
assumed, and both narrowed what an answer could look like:

1. **There is no CLI-to-supervisor socket at all.** `ADR-017` §3.4's
   `NOTIFY_SOCKET` is `sd_notify`'s: per-service, created by the supervisor,
   read in the *service → supervisor* direction. It is the only `AF_UNIX` socket
   in this repository.
2. **The supervisor's service set is fixed at the moment it starts.** Nothing
   re-reads the definitions, the applications or the document after the
   `Supervisor` is built, so a request written anywhere it could see would not
   be noticed until the next boot.

**Every candidate appeared to overturn something already ruled** — a socket is a
protocol, a request file plus polling changes what `ADR-017` §6's `enabled`
means, and spawning the installer detached from the CLI abandons supervision and
`ADR-022` §4's capture, which is what `rivet get install` reads. That made it
`AGENTS.md` §1's case rather than a matter of effort, so `cmd/rivet` refused
`apply install` on every platform, naming what was missing and where the
options lived (the maintainer's 2026-08-24 install-request ruling).

**One clause of that was wrong, and finding out cost nothing because the ruling
was asked for rather than guessed at.** *"A request file plus polling changes
what `enabled` means"* is true only of the cheapest implementation of a request
file — the one that compiles the installer into `ADR-017` §4.1's list disabled
and has the request flip the flag. `ADR-017` §3.2.2 does not do that: it creates
a service **instance** no document names, so `enabled` keeps exactly one writer
and §6 is untouched. The other two clauses stand, and the third is why option C
stayed refused. **`cmd/rivet` no longer refuses `apply install`**; it delivers
a request and returns before a byte is written, which is the maintainer's
2026-08-24 ruling's whole purpose.

#### 14.7.3 A defect found in this binary while wiring the report

`install::Outcome::wrote_nothing` put `RefusedMachine` in the *wrote nothing*
arm, so `main` emitted `wrote: false` on **every** machine refusal — including
one arriving part-way through a 640 MiB write, which is the only case in this
binary where an inactive slot is left neither old nor new.

`AGENTS.md` §5 is the rule — *report what the system said, not what it was
expected to say* — and the direction is the dangerous one: the record an
operator reads after a failed install asserted that the slot was untouched. It
now answers `None` for that arm, the field is **omitted** rather than guessed
(`ADR-006` §8.6.3 already ruled that absence is how a value says it has none),
and the message says the records above it are what say how far the write got.
The other three arms are promises §5's ordering makes structural and are
unchanged.

**A second, smaller one**: a dry run emitted no terminal record at all, so its
chain simply stopped. Both were found by building a reader for these records
rather than by reading the writer, which is `AGENTS.md` §4's *"prioritise the
tests that tie two modules together"* arriving as a fact rather than as advice.

---

## 15. The builder — 2026-08-24, and the hole it closed

§14 built the thing that **installs** a payload. **Nothing built one.** This
section is that half, and it starts with the gap rather than the feature,
because the gap is what makes the section worth reading.

### 15.1 The hole, and why it was invisible

Verified two ways before it was believed (`AGENTS.md` §7): `cmd/cargo-rivet`
had exactly two verbs, `package` and `inspect`; and no crate in the workspace
named `Kind::update()` outside `rivet-core`, its own tests, and two `#[ignore]`d
benches.

> **`cmd/rivet-deploy` could install a system update that no tool could
> produce.**

Its whole install path — §9.2 steps 1 through 10, `InstallPlan`, the
skip-if-identical safety property, the F5 warning — had only ever run against
fixtures the *test harness* assembled: `rivet-crypto/tests/loopback.rs` and
`cmd/rivet-deploy/src/bench.rs`, each of which builds a payload by hand and
each of which says in its own comments that it is not the authority for what a
card contains. §14.6's run is real and is not diminished by this; what it could
not settle is whether anything a person could *invoke* produces bytes of that
shape.

**That is the kind of gap this project keeps designing against and had grown
anyway.** A format is not deployed because one end reads it. The `.rvt` had a
verifier with full scrutiny (§11), an installer with a loop-device run (§14.6),
and no producer at all.

### 15.2 Where it went, and why not where §10 said

§10's table gave the emitter to `cmd/rivet-image`. **That row is superseded**,
and the reason is one word: **keys.**

`cmd/cargo-rivet` is the **only crate in this workspace that holds a signing
key**, which is its defining property rather than an accident of what it does
today — `ADR-015` §4's first bullet is that the private key never leaves the
build machine, and `cmd/cargo-rivet` is the build machine's tool. Putting an
emitter in `cmd/rivet-image` would take one of two shapes and both are worse:

| | Cost |
| :-- | :-- |
| It signs | **A second signing-key holder in the workspace.** Two places that read a private seed is two places to get key handling wrong, and `PLAN.md` §0.4.2 has not ruled key handling once |
| It emits unsigned, and a second tool signs | A pipeline with an **unsigned artifact in the middle**, which is a thing a build script can ship by accident |

§10's row was written on 2026-08-23. `cmd/cargo-rivet` was created on
2026-08-24. **The row named the only crate that could have done it at the
time**, which is why it is struck rather than called wrong.

**The half of that row that survives is its observation about erofs**, and it
now applies to this verb's input rather than to its location: once §3.1's ext4
stopgap retires, `disk::mkfs_erofs_args` produces a standalone `/system` file
that is `--system`'s argument directly; the ext4 case builds in place at an
offset and must be extracted (a loop partition, which is how it was exercised
below).

### 15.3 What the input is — the decision that shaped the verb

`ADR-018` §5.2 and §9.2 steps 7–8 constrain it exactly: **a payload is what one
slot needs.** So:

```
cargo rivet update --system PATH --boot-a DIR --boot-b DIR
                     --signing-key PATH --version VERSION --out-dir DIR
```

* `--system` is the `/system` **filesystem** — a file, or a partition device.
  Streamed past the digest and never resident, because it is 640 MiB.
* `--boot-a` / `--boot-b` are the two boot partitions **as files**.

**It deliberately does not take a card image.** Slicing one means a GPT parser
and a FAT reader inside the tool that holds the signing key — two parsers over
somebody else's bytes, in the one program §5 spent its whole design keeping
parsers *out* of — to save a `losetup` the operator runs anyway. What a card
image is, is `cmd/rivet-image`'s; what a payload is, is this.

### 15.4 The slot variance is measured on every build, not declared

§3's `slot_variance` rests on a measurement: *"extracting `p1` and `p2` from a
built image and diffing them, `cmdline.txt` is the only file that differs."*

**Both existing fixtures encode the conclusion** — a hard-coded list of six
invariant names plus `cmdline.txt` — and both say in their own comments that
the list will go stale, because what a card carries is `cmd/rivet-image`'s to
decide.

**This verb does the diff instead.** A name whose bytes agree across the two
directories becomes one `Invariant` member; a name whose bytes differ becomes
two. Nothing in the code knows that `cmdline.txt` is special. Three consequences,
and the third is the one that matters:

1. A release in which a second file becomes slot-variant produces a **correct**
   payload rather than a silently wrong one.
2. The report prints what it found, so §3's measurement is **re-taken on every
   build** instead of quoted from this file.
3. When the finding disagrees with §3 — anything other than `cmdline.txt` alone
   — the tool prints a warning saying **this ADR is stale.** A document that can
   tell you it has gone out of date is `AGENTS.md` §7 mechanised rather than
   exhorted.

A name present on **one** side only is refused, naming the file and the slot
that lacks it. `Payload::open` refuses it on the board anyway
(`SlotVarianceIncomplete`); refusing here names the file while it is still on the
build host's disk.

### 15.5 The two facts the builder is not allowed to know

Both come from `BootSlots`, which is why `cmd/cargo-rivet` now depends on
`rivet-adapter-image` (`ADR-014` §3 permits `cmd/*` → adapter, and this is a
composition root):

| Fact | Why it is the port's |
| :-- | :-- |
| `<platform>` | `platform_token()` — §9.2 step 2 compares it against a signed header on the machine. A second spelling of `rpi4` in a builder is a payload no board accepts (`ADR-011`) |
| §4's reserved names | `reserved_names()` — on the Pi a pair of filenames, on a UEFI machine an EFI variable that is not a file at all |

`--platform` therefore accepts only a token with an implementor, and the refusal
names the ones `ADR-027` §3.1 lists that have none (`rpizw`, `uefi`) rather than
producing a payload for a platform nothing can describe.

**Reserved names are skipped and every skip is printed.** A real boot partition
carries both of them, so dropping them is not optional — and §4's own argument
against silence is that *"silently skipping would let a future builder believe it
had shipped something it had not."* `Builder::reserving` is given the same list
as a backstop, so a name that escaped the filter is refused rather than shipped;
that backstop was watched firing.

### 15.6 What `--version` does NOT do, and it is the interesting refusal

`cargo rivet package` composes a version from the crate's own and `git`
(`ADR-027` §3.2, §3.2.1). **`cargo rivet update` refuses to.** It is handed a
system image it did not build, so composing `-g<sha>` here would make §3.2.1's
claim — *"these bytes came from commit X"* — **on somebody else's behalf**, in
the direction that is hardest to notice because the name looks exactly like a
name that is true.

So `--version` is required with no default, and the report says the string was
taken as given. It is the same posture as `--signing-key` and `--out-dir`: three
flags naming three decisions the tool declines to make (`AGENTS.md` §1).

### 15.7 One defect this found in the tool beside it

`cargo rivet inspect` called `Application::open` **unconditionally**, so a
perfectly valid update payload was refused by the *reporting* layer after the
verifier had already accepted it. §11's argument is one container and a per-kind
interpretation; that was the one place with the interpretation hard-coded.
`inspect` now dispatches on the header's `kind` and refuses an unimplemented one
by name — and it reports, for an update, which member goes to which slot, taken
from `Payload::boot_members_for` rather than from the member table, so what is
printed is the selection a board makes.

### 15.8 What has been run, and on what — `AGENTS.md` §6

**A dated observation about a tree, not a claim that survives being read
later.**

**A real 683 631 182-byte payload was built from a real card image** — one of
`out/`'s 2 GiB cards, attached with `losetup -r` (**read-only**, so nothing in
this run could write to it) and both boot partitions mounted `-o ro`. The
`/system` member was streamed straight off the loop partition; the boot half was
the card's own nine files.

| What the run produced | |
| :-- | --: |
| `system.img` | 671 088 640 |
| `kernel8.img` | 10 168 774 |
| `start4.elf` | 2 305 632 |
| `bcm2711-rpi-4-b.dtb` | 56 407 |
| `fixup4.dat` | 5 499 |
| `config.txt` / `tryboot.txt` | 931 each |
| `cmdline.txt` × 2 | 136 each |
| **total** | **683 631 182** |

**Every number is §6's own table**, and the total is the same 683 631 182 bytes
§14.6 records for the payload its bench built — which is corroboration rather
than a coincidence, and is the closest thing available to testing one side
against bytes the other side wrote.

Also observed in that run, and each is a claim in this document being re-taken
rather than repeated:

* `autoboot.txt` and `rivet.toml` were on the card, were **not packaged**, and
  were named in the report — §4.
* Exactly **one** file differed between `p1` and `p2`, and it was `cmdline.txt`
  — §3's measurement, third independent confirmation.
* `cargo rivet inspect` verified the whole 683 MB file back through
  `rivet_core::package::read` — **the same verifier a board runs** — and
  `Payload::open` accepted it as a complete update for both slots.
* A single byte flipped inside the `/system` member was **refused**, naming
  member 0 — §5 step 8.

In the committed suite: the round trip goes through the real verifier, through
`Payload::open` with §4's names taken from `PiFirmware`, and through
**`rivet_adapter_image::install::InstallPlan`** — the shipped installer's
*decision* half — for both slots, asserting the system-before-boot ordering, the
skip-if-identical action, F8's refusal and F5's `targets_the_fallback_partition`
being true for slot A and false for slot B. Five defect injections were run and
each was watched failing the tests named for it.

**What none of this settles**, and this list is the important half:

* **Nothing here has been installed.** `cmd/rivet-deploy` was **not** driven at
  this payload. That rig — §14.6's — needs `/dev/mmcblk0p1`–`p4` synthesised as
  device nodes and `/proc/device-tree/chosen/bootloader` bind-mounted inside a
  mount namespace, because the installer's device paths are constants; it was not
  rebuilt here. **What is proven is that the payload satisfies every refusal on
  the path up to the first byte of I/O, and the plan that orders that I/O.**
* **Nothing has run on a Raspberry Pi**, and this changes nothing about that.
  §14.6's list of what a loop device cannot be applies unchanged, and this run
  did not even write to one.
* **No Rivet can verify one of these**, because no image carries a root of
  trust — `ADR-028` §5.2, `PLAN.md` §0.8 link 4. The trust set in every run above
  is one key on a command line, which is a developer checking their own artifact
  and is not a fleet.
* **The signing key story is unchanged and unruled** — `PLAN.md` §0.4.2. This
  verb takes a path and picks no default, exactly as `package` does.
* **`PRODUCT` is now a constant in `rivet_core::package::update`** and the
  older spellings were **not** migrated: `cmd/rivet-image`, `cmd/rivet-deploy`
  and `rivet-crypto`'s bench each still carry their own. That was deliberate —
  they are crates this change did not own — but it means the constant is an
  authority in waiting rather than an authority, and it is named here so the
  migration is a task rather than a discovery.

---


## 16. RULED 2026-08-29: the docs ship on the card, as the manual for that instance

**Moved here from `PLAN.md` §101 on 2026-08-29, unaltered.** Ruled by the
maintainer, Joel Lindell, the same day. Only the heading numbers changed.
**Bare `§N` references in the body are `PLAN.md` sections** — `§85` is
`git-http-backend` running on a card, and `§98.2` / `§98.4` are the WARNING
doctrine, now `ADR-012` §7.4.2 and §7.4.4.

**It belongs in this ADR because it is a MEMBERSHIP ruling.** The manual is a
member of the container §3 lays out, covered by §5's single signature and
verified by §9.2 before anything is written. That is what makes a stale manual
structurally impossible rather than merely unlikely, and it is the whole
argument — §16.1 makes it explicitly as `ADR-012` §7.4.4's preference order
applied: structure, not warning.

**What §16.5 produced, so the artifact is traceable to the measurement.**
`tools/docs/index.py` and `docs/INDEX.md` exist because of it — a **section**
index and not a search index, generated from the same commit, with `--check` as
the drift guard §16.5.2 demands. §16.5 also produced the observation that split
`PLAN.md`: one file was 40% of the corpus.


**The maintainer, having noticed the size: the code and the `./docs/` folder are
size-wise peanuts in this project, which made him wonder whether baking in at
least the docs — and perhaps the code too — as a searchable reference for the
MCP could be enormously valuable in the long run. He asked whether he was over-
imagining it.** *(`.rulings/` R-047.)*

**He is not, and the numbers are not close enough to argue about.** Measured
2026-08-29 against the live board:

| | |
| :-- | :-- |
| `docs/` | 37 files, 41,694 lines — **0.8 MiB gzipped** |
| Rust source | 346 files, 168,932 lines — **1.9 MiB gzipped** |
| the running system | **56 MiB used of a 613 MiB slot** — 10% |

The docs are **0.13% of the slot**, and `/system` is `erofs`, so they land
compressed regardless.

### 16.1 RULED: a member of the signed payload, not a file copied onto a card

> ship the docs as a member of the signed `.rvt` payload — structurally
> read-only — this is the "manual" for this instance of Rivet OS.

**The framing is the ruling: it is the manual for THAT INSTANCE.** Which settles
the one real objection, because a design record on a card that does not match
the running binary is **worse than none** — it answers confidently and wrongly.

That is not hypothetical. It happened today, in this file's own subject matter:
`ADR-029` §9.3's install warning said *"the mitigation is not ruled and this
build does not choose one"* for an hour after it was ruled and built, and
anybody reading the machine's own words would have been misled by them (`ADR-012` §7.4.2).
A stale manual is that failure with a much larger surface.

**Payload membership is what makes drift impossible rather than merely
unlikely**, and it is `ADR-012` §7.4.4's preference order applied — structure, not warning:

* `ADR-029`'s format already carries members with digests under **one
  signature**, and §9.2 verifies every member's bytes before anything is
  written. The manual arrives with the binaries, at one commit, atomically, or
  not at all.
* `/system` is read-only **by format** (`ADR-018` §3), so "structurally
  read-only" is free rather than enforced — there is no write path for a root
  process to take.
* The payload already carries `version`, so the manual is **self-labelling**.

### 16.2 Docs and code are not the same case

Ruled together, but for different reasons, and conflating them would be sloppy:

* **Docs answer *why*, and that is the thing a running machine cannot
  reconstruct.** It is also the thing an agent needs most: this session spent
  much of a day reading `PLAN.md` and the ADRs to answer questions, and an agent
  handed a board and nothing else currently cannot do that at all. `ADR-032` is
  titled *Asking a Running Machine*; this is that instinct with the interesting
  half added.
* **Code's value is provenance, not reference.** The binary already *is* the
  "what", and source on the card does not prove correspondence to it. What it
  buys is a machine that can hand you the exact source of what it is running,
  signed, at a known commit — which matters for a fleet member nobody can walk
  to.

### 16.3 `ADR-011`: the repository stays authoritative

The card carries a **labelled snapshot**, and must say so. Stated now because
otherwise somebody later treats a board as a source of truth about the design,
which is exactly the second-authority problem `ADR-011` exists to prevent.

### 16.4 Not yet decided

* **How it is served.** The board already serves git with a vendored
  `git 2.53.0` — `cmd/rivet-mcpd/src/git.rs`, smart HTTP on the TLS listener
  (`ADR-035` §4) — and §85 proved `git-http-backend` runs on a card, so the
  machine could **host its own repository** and be cloned from over the same
  TLS control plane. That is more than "a searchable reference" and may be the
  better shape, but `gitd`'s repositories live in `/state` (which is writable
  and is user data) while the manual belongs in `/system` beside the binaries.
  The two are not obviously the same mechanism.
* **What "searchable" means as an operation.** A `get` subject over the manual
  is not the same thing as `git clone`, and `ADR-006` §6.4's verb set is closed.
* **Whether the source ships in the same payload or a separate optional one.**
  1.9 MiB is nothing, but an optional member is a different `ADR-029` question.

### 16.5 MEASURED: the manual needs no search index, and the reason is not the one expected

The maintainer asked whether indexing or pre-caching would make MCP queries
against §101's manual faster. **Measured on the board rather than estimated**,
2026-08-29, reading from `/system` — which is `erofs`, so this is the real path:

| | Read | Byte scan |
| :-- | :-- | :-- |
| 839 kB, cold | **3.1 ms** | **2.6 ms** |
| 839 kB, warm | **2.2 ms** | — |
| 44.5 MB, cold | 687 ms | 452 ms |

The manual is 2.4 MB. At the representative scale, **reading and scanning the
whole corpus costs on the order of 20 ms cold**, and the page cache takes the
read to ~2 ms warm. **Pre-caching is already solved by the kernel**, and `erofs`
means it is stored compressed and cached decompressed at no cost to anyone.

An index would save perhaps 10 ms against a process spawn, MCP framing and JSON
serialisation that each cost more than the search does. **It optimises the wrong
resource.**

#### 16.5.1 The real scarcity is the QUERYING AGENT'S CONTEXT, not the board's CPU

A grep returning 200 matching lines across 37 files costs the asking agent
enormously more than it costs the Pi. So the useful artifact is not a search
index but a **section index** — heading to file and line range, a few kilobytes
— because of a property this corpus already has:

**These documents are rigorously structured, and this project already cites by
section and never by line.** Every commit message of 2026-08-29 says `ADR-029`
§9.3 or §98.4. An index of headings lets a query answer *"`ADR-029` §9.3,
`ADR-018` §5.2"* and the caller then fetch exactly those sections — which is how
a human uses this record and how the history already references it.

**And one file dominates.** `PLAN.md` is ~17,000 lines, roughly **40% of the
whole corpus**. Grepping it returns punishing amounts of context for one answer.
**Sectioning that file is worth more than indexing everything else put
together**, and it is a candidate for being split — which is a decision about
this file and therefore the maintainer's.

#### 16.5.2 The constraint, which is §16's constraint again

An index is a **second representation of the same content**, which is `ADR-011`'s
problem and precisely the drift §16.1 closed. So if one exists it must be
**generated at payload-build time from the same commit and shipped as a member
of the same signed payload** — never computed on the board, never separately
updatable. Same rule as the manual, for the same reason.


### 16.6 RULED 2026-08-29: what ships is `docs/`, and the maintainer's own words are not in it

§16.1 makes `docs/` a member of the signed payload — the manual for that
instance of Rivet. **This section draws the boundary of that member**, and it
does so now rather than when the packaging is built, because the cheapest moment
to decide what a distribution contains is before it has ever contained it.

**The ruling:** a verbatim quotation of the maintainer lives in
`.rulings/RULINGS.md` and **nowhere inside `docs/`**. Each is numbered `R-NNN`.
A `docs/` file states the ruling in its own words and cites the id.

**`.rulings/` is outside `docs/` on purpose, and that is the load-bearing
half.** *"The manual is `docs/`"* then stays a simple true sentence, and the
exclusion is structural rather than a file inside the shipped directory carrying
a note asking not to be shipped. That is `ADR-012` §7.4.4's preference order —
structure over warning — applied to packaging.

**It is a dot-directory rather than a file at the repository root because the
maintainer asked for one**, 2026-08-29, to keep the workspace root to the files a
newcomer should read first. *(`.rulings/` R-100.)* A dot-directory is
his established idiom for a tool's own record. The directory holds one file today
and exists so that a later split — by year, by subject — is not a second move.

**A dot-directory *inside* `docs/` would have satisfied the tidiness and broken
the ruling**, since it would still ship. Recorded because it is the obvious
wrong turn.

**Its path is `.rulings/RULINGS.md`, amended 2026-08-29.** It began at the
repository root the same day and was moved into a dot-directory at the
maintainer's request, so that the workspace root stays uncluttered. Only the
location moved — the scheme, the numbering and the rules are as ruled above, and
`docs/` cites it as *(`.rulings/` R-0NN.)*

#### Why separation rather than deletion

The quotations are the evidence that a ruling happened, when, and in whose
words. That is the difference between *"the maintainer decided X"* and *"an
agent decided X and attributed it"*, and this record leans on that distinction
in thirty documents. Deleting them would destroy provenance permanently in order
to solve a distribution problem.

**And it inverts which mistake is easy to make.** Before: publishing safely meant
finding every quotation scattered through the corpus and being certain the last
one had been found — a task with no completion test, which is `AGENTS.md` §7's
shape exactly. After: publishing safely means not copying one file.

#### The rule that actually costs something

**A section that cites `.rulings/RULINGS.md` must read completely without the citation.**
The file is not on the card, so a reader on a board cannot follow the pointer —
and §16.1's whole argument is that a manual answering confidently and wrongly is
worse than no manual. A dangling citation whose content was load-bearing is that
failure.

So a migration is not a substitution. Each quotation's citing section has to be
rewritten to carry the ruling in its own prose first, and only then does the
quotation move. That is judgement, which is why `tools/docs/rulings.py`
deliberately does not check it — a checker that guessed would produce exactly the
confident-and-wrong output this project's guards exist to find.

#### State, measured 2026-08-29

`ADR-035` is migrated — four quotations, `.rulings/` R-001 to R-004 — and is the
worked example. **`uv run tools/docs/rulings.py` reports 76 further blockquotes
in `docs/` that are attributed to the maintainer and not yet recorded.** That
number is the tool's, not this sentence's; re-run it rather than citing this.

The guard's exact half **fails** if a recorded quotation reappears in `docs/`,
comparing on a normalised form so a re-wrap is not a hiding place. Its fuzzy half
**reports only**, because a heuristic that blocks a commit is a heuristic that
gets switched off — `AGENTS.md` §10.5's twelve clippy errors are the worked
example of a routed-around gate.

#### Not settled by this

* **Whether anything else is excluded from the manual.** `PLAN-Archive-*` is a
  dated build journal rather than design; whether a card wants five archive files
  is a real question and is not this ruling's.
* **The public scrub is broader than this file.** Commit messages, `AGENTS.md`
  and source comments also quote him. This settles the *manual*, which is what
  ships to every board; a public repository is a separate exercise with its own
  boundary.

---

## 13. RULED 2026-08-29: payloads stay uncompressed, and retention is the fix

§12 left the decompressor unchosen. **Ruled: none. Payloads stay uncompressed,
and the disk problem is solved by deleting old ones.** *(`.rulings/` R-107.)*

Three options were put — `zstd`, `xz`, or leave it uncompressed and delete —
and the third was recommended and taken.

### 13.1 The problem was accumulation, and accumulation now has a fix

**Nine payloads, 4.8 GB, had collected on the board** because nothing on the
machine could delete them. That is the operational case that ruled file
management (`ADR-006` §8.10) the same morning, and `apply remove-file` exists
now. **The disk pressure was never compression's problem to solve.**

### 13.2 What a decompressor would actually cost

* **A permanent dependency in the image, on the device that boots it.** Every
  machine carries it for ever, to save transfer time on an operation that
  happens rarely and is not interactive.
* **The win is smaller than 683 MB suggests.** §6.1 already records that the
  kernel should be *stored* uncompressed, so a meaningful share of the payload is
  content that would be decompressed and re-expanded on the card anyway.
* `xz` was rejected on its own terms: ~2.5% better ratio than `zstd` for
  materially slower decompression on a Pi, which is the wrong end of the trade
  on this hardware.

### 13.3 The condition that reopens this

**Transfer time over a link that is not a LAN.** Compression is a *network*
optimisation and the argument against it is a *device* argument; the first Rivet
reached over something slower than the local network is where the two swap
places. Written here so the trigger is a recorded condition rather than somebody
remembering — `PLAN.md` §94.7.

### 13.4 What this implies and is not yet built

**A retention policy on the machine.** §12 tied compression to how long payloads
are kept, and ruling one does not rule the other: nothing on a board expires an
old `.rvt` on its own, so *"delete them"* is currently an operator's job that an
operator has to remember. The card build already prunes on the workstation side
(`RIVET_KEEP`); the board has no equivalent. **That is open and belongs in
`PLAN.md` §102.4.1 rather than being quietly assumed solved by this ruling.**

### 13.5 RULED 2026-09-13: a `card` image is the one kind that compresses — `R-177.1`

**This does not reopen R-107.** The reasoning above is a *device* argument —
a decompressor is a dependency the machine that boots it carries forever — and
it stands unchanged for every kind a board installs: `update`, `application`,
`library`. A `card` image is never installed by a board at all; `cargo rivet
image` decompresses it exactly once, on the workstation, when provisioning
writes a device, so no board ever links a decompressor for it. The two
questions look similar and are not the same question, and `R-177` is explicit
that this section was checked before the codec was chosen rather than assumed
not to collide with it.

**The codec is zstd** — `R-177.1`, the same choice §13.2 already argued for on
this hardware (*"`xz` was rejected on its own terms: ~2.5% better ratio…for
materially slower decompression"*), measured again for a 2 GiB card image
rather than a ~700 MB update: much faster to compress and decompress, within
a few percent of `xz`'s size.

**What this changes in the format.** `M_COMPRESSION` (§3's member record,
byte 65) gains its first non-zero legal value,
`rivet_core::package::read::COMPRESSION_ZSTD` (`1`). §5.1's *"the decompressor
only ever sees bytes the signer produced"* is exactly as true for this value
as for `COMPRESSION_NONE`: the digest in a member's record is still over the
**stored** (compressed) bytes, checked before any of them reach a
decompressor, and this crate's own reader never runs one — decompression, like
the codec's encoder, lives in `cmd/cargo-rivet` alone, the one binary that
ever needs to run it. §12 item 2, *"a decompressor"* singular for the whole
format, is answered exactly this narrowly: one codec, legal on one kind.

### 16.4.1 RULED 2026-08-29: plain files and a line range on `get file`

§16.4 left *how the manual is served* open. **Ruled: it is not served. It is
read** — the payload puts `docs/` on the card as plain files, and an agent
fetches a section with `get file` plus a line range. *(`.rulings/` R-112.)*

**Every piece already exists and is already ruled.** `get file` is built
(`ADR-006` §8.10); `docs/index.json` maps every heading to a file and a
**1-based inclusive line range**, which §16.5 says exists precisely so a section
can be *fetched* and not merely located. The two compose into the whole feature
with nothing new.

Two alternatives were put and both add mechanism to a solved problem:

* **The git server serves it**, so the manual can be cloned off the board. The
  attraction is real and §16.4 named it, but git's repositories live in
  `/state` — writable, and user data — while the manual belongs in `/system`
  beside the binaries it documents. That is two storage models for one artifact.
* **A dedicated `get` subject.** `ADR-006` §6.4's verb set is closed, and a
  subject that reads a file when `get file` reads files is a second authority
  over the same act.

**A line range is the whole of what was missing**, and `get file` may not have
one yet — §102.4.2 records `--follow` and drift as the shape of "specified,
unbuilt". If it does not, that range is this ruling's only new code.

**Not built, and not measured on a card.** §16.5's 20 ms cold figure was taken
over `erofs` on a board and is the reason no search index is needed; nothing has
yet fetched a manual section from a real machine.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.
§14's blockquote about the `cmd/rivet-deploy` rename keeps its inline strike for
the same reason: the old crate path has to be spelled to be corrected, and the
strike is what stops the checker reading it as a claim that the directory
exists.

### The magic lost its end-of-file byte with the rename to Rivet

§3's table drew the magic as ~~`"RIVET\x1A"`~~ long after the code had moved
to `RIVETPKG`. The original magic was ~~`FERRIPI\x1A`~~: a name padded to eight
bytes with DOS end-of-file, PNG's trick, so a payload printed to a terminal
stopped at byte eight. The rename to Rivet (`ADR-033`) left `RIVET` three bytes
short, and the maintainer ruled all eight bytes printable rather than a name
plus padding, trading the terminal-stopping property for a hex dump that names
the file in full. No compatibility branch exists: an old-magic payload fails at
byte zero, which is the correct refusal for a format this reader does not
claim to read.

### In §2 — nothing in this ADR was implemented

**Built 2026-08-24 — §13, §14 and §15.** The superseded sentence read:

~~**Nothing in this ADR is implemented.**~~

§13.4's *"nothing writes to a slot in shipped code"* went stale within hours of
being written.

### In §7.2 — how to get BLAKE3's pure-Rust mode

**Corrected 2026-08-23**, and the correction matters because the wrong version
looks like it works. The superseded sentence read:

~~`default-features = false` gives pure-Rust mode.~~

It only drops `std`. Without the `pure` feature a Rivet build compiles
`c/blake3_neon.c` for the board and needs the C cross-toolchain the paragraph
exists to avoid.

### In §9.3 — the F5 mitigation was unruled

**Ruled 2026-08-29 by the maintainer: detect and repair; built the same day.**
The superseded text read:

~~**Three candidate answers, none yet ruled**~~

~~Until one is ruled, **`ADR-018` §5.2's flow should not be trusted for an update
that targets slot A from a machine running slot B.**~~

### In §10 — the payload emitter was to live in `cmd/rivet-image`

**Superseded 2026-08-24 — §15.** The superseded row read:

~~**`cmd/rivet-image`**~~ | ~~Gains a payload emitter and composes its own name
(`ADR-027` §7). `disk::mkfs_erofs_args` already builds erofs to a standalone
file, so once §3.1's ext4 stopgap retires the system member is produced by the
existing path; the ext4 case builds in place at an offset and needs extraction~~

The emitter is `cargo rivet update`, because `cmd/cargo-rivet` is the only crate
in the workspace that holds a signing key and it did not exist when the row was
written.

### In §12 items 3 and 4 — two questions that closed

**Item 3 closed on hardware 2026-08-23 and its follow-on was ruled 2026-08-29;
item 4 was ruled by the maintainer 2026-08-24.** The superseded text read:

~~**What the Pi firmware does with a *corrupt* `autoboot.txt`.** Needs a
hardware experiment; it sets how bad F5 is.~~

~~**which of §9.3's three mitigations.**~~

~~**Which process installs an update.** `ADR-028` §4 rules the *authority* and
not the executor. `cmd/rivet` was ruled the **arming** caller on
2026-08-23; whether it is also the installing caller is not the same
question, and the answer decides whether `rivet-adapter-image` ships a
`BootMedium` implementor of its own.~~

### In §13.4 — the installer and the emitter did not exist

**Both superseded later on 2026-08-24 — §14 and §15.** The superseded bullets
read:

~~**The installer.** §12 item 4 — *which process installs an update* — is
open, so nothing writes to a slot in shipped code.~~

~~**A payload emitter in `cmd/rivet-image`, and a CLI verb in `cmd/rivet`.**
Neither exists; the writer and the verifier they would call do.~~

The first stayed true for hours. The prediction inside it turned out right and
is kept in the body.

### In §14.4 — no operator could start the installer

**The channel was ruled 2026-08-24 evening — §14.7.2.** The superseded clause
read:

~~**there is no way today for an operator to start this binary with a path
through the supervisor at all**~~

### In §14.7.2 — the channel was open, and every candidate looked blocked

**Ruled the same evening it was written.** The superseded heading was ~~Still
open: the channel~~, and the superseded text:

~~§14.4 said there is no way to start the binary with a path through the
supervisor.~~

~~**Every candidate therefore overturns something already ruled** — a socket is a
protocol, a request file plus polling changes what `ADR-017` §6's `enabled`
means, and spawning the installer detached from the CLI abandons supervision and
`ADR-022` §4's capture, which is what `rivet get install` reads. That is
`AGENTS.md` §1's case rather than a matter of effort, so `cmd/rivet` **refuses
`apply install` on every platform**, naming what is missing and where the
options live (the maintainer's 2026-08-24 install-request ruling).~~

One clause of that was wrong, and finding out cost nothing because the ruling
was asked for rather than guessed at.
