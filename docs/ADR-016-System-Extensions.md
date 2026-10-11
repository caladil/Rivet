# ADR-016: System Extensions

* **Status:** Accepted — ruled 2026-08-22.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-005` §5.5 ruled that `helix` would be an "add-on image variant", and
`ADR-008` §4.1 defined an add-on as a build-time image variant. Following that
literally produces an absurd conclusion: **rebuilding and reflashing the
operating system in order to get a text editor.**

**The maintainer identified the problem and the shape of the answer, 2026-08-22:
could the image be layered so that `/system` remains the immutable system while
providing a *mount point* beneath it — he offered a candidate name and said it
needed a better one — for a layer of system-installed packages? He asked whether
that was a crazy bad idea.** *(`.rulings/` R-035.)*

It is not a crazy idea. It is **Android APEX**, and `systemd-sysext` is the same
concept on ordinary Linux: signed, versioned, read-only images mounted into an
otherwise immutable operating system. `ADR-008` §4.1 had already named APEX as
the direction if image variants proved too coarse — helix is the case that
proved it.

---

## 2. Decision

**A system extension is a signed, versioned, read-only image mounted at
`/system/ext/<name>`.** It extends the base image without being part of it and
without being rebuilt with it.

It is the middle row of `ADR-015` §2's three categories, and it exists because
neither neighbour fits:

* **Not the base image**, because tooling should not force an OS rebuild and
  reflash.
* **Not an application** (`ADR-015`), because sandboxing an editor away from the
  filesystem it exists to edit defeats the point. Extensions are *part of the
  system's trust domain* — they are for software the operator trusts: editors,
  compilers, debugging tools.

### 2.1 The name

**`/system/ext`**, ruled 2026-08-22. "Extension" is what both Android and
systemd call this, and it avoids colliding with *application*, which already
means a sandboxed user program under `ADR-015`. Two things sharing one word is
what `AGENTS.md` §10.2 exists to prevent.

---

## 3. Separate mounts, not a merged overlay

Two mechanisms were available and the choice matters more than it appears.

| | Merged overlay | **Separate mounts** |
| :-- | :-- | :-- |
| Where binaries appear | one merged `/system/bin` | `/system/ext/<name>/bin` |
| `PATH` | unchanged | composed by Rivet |
| "Which extension owns this file?" | **unanswerable** | always answerable |

**Separate mounts, with Rivet composing `PATH`.** The merged overlay is more
seamless and makes file ownership indeterminate, which is precisely the second
authority `ADR-011` forbids. When two extensions provide a binary of the same
name, an overlay silently resolves it by layer order; separate mounts make the
collision visible and force it to be decided.

Implementation is an ordinary read-only image — `squashfs` or `erofs` — mounted
at boot. `dm-verity` is the stronger option for integrity and is what APEX uses;
signature verification of the image (§5) is the minimum.

**Located 2026-08-22 by `ADR-018` §6:** extensions are files, not partitions —
`/state/ext/<name>.img`, loop-mounted at `/system/ext/<name>`. They live in
`/state` because they must survive a `/system` replacement, which is precisely
what an update never touches. Note the detail that makes the first one possible
at all: mount points cannot be created on a read-only filesystem, so
`/system/ext` is a **tmpfs** over an empty directory in the image.

---

## 4. The guard rails that stop this becoming a package manager

`ADR-008` §4.2 withdrew "no package manager, *ever*" but kept the property that
matters: software may never be installed such that the machine diverges from its
document. An extension mechanism is the most likely way to lose that by
accident, so three rules define the boundary. **If all three hold, this is an
image mechanism and not a package manager.**

1. **An extension is a whole signed image, never individual files.** There is no
   "install this binary into a shared tree".
2. **Extensions have no dependencies on each other.** No resolution, no ordering
   constraints, no version solving. An extension that needs something brings it.
3. **Extensions run no install scripts.** Mounting is the entire installation.
   There is no pre-install, post-install, or trigger.

The first rule is the load-bearing one. The moment individual files can be
placed into a shared tree, ownership becomes ambiguous and §3's argument
collapses along with it.

---

## 5. Extensions are configuration

Like applications (`ADR-015` §2.1), **the set of installed extensions is part of
the desired-state document.** Otherwise there would be a category of installed
software the document cannot describe, which is the second authority the record
exists to prevent.

So extensions are declared and `rivet apply` mounts them; `rivet export`
reports them; `plan` shows one arriving before it does.

**Signing and delivery are shared with `ADR-015` §4** — the same trusted-key
set, the same verification, the same transport. This is `ADR-008` §4.2's third
commitment paying off: one artifact-delivery path serves image updates,
extensions and applications, and none of the three needed its own.

---

## 6. Consequences

* **`ADR-005` §5.5 is amended.** `helix` is a **system extension**, not an image
  variant. The base image still ships a small editor; helix becomes the obvious
  first extension.
* **A development toolchain becomes tractable.** `ADR-015` §10 leaves open
  whether a Rust toolchain on-device is ever wanted. If it is, it is an
  extension rather than an image variant or a sandboxed application — which is a
  much better answer than either.
* **Image variants remain, for a narrower job.** `ADR-008` §4.1's variants are
  still how genuinely different *base* images are produced. Extensions handle
  additions to a base; variants handle different bases. Do not use one for the
  other's job.

  **Amended 2026-08-22** (`PLAN.md` §4.13, ruled by the maintainer): this bullet
  previously named "the headless and GUI images" as the example of a genuine
  variant. **That example is withdrawn — the GUI is an extension.** See §6.1.
  The bullet's principle stands; it has simply lost its only concrete instance,
  and no other candidate for a genuine base variant has been identified.

### 6.1 The GUI is the first extension that pays for the mechanism

**Ruled 2026-08-22** (`PLAN.md` §4.13). The maintainer's answer was not one of
the three offered but a path through two of them: **a kiosk surface first — one
compositor, one application — with a fuller desktop later, as a configuration
option or a further extension**, drawing heavily on COSMIC.

That shape has a consequence worth stating plainly, because it is what makes the
decision more than a preference: **extensions must layer, not merely exist.** A
desktop is not a different extension from a kiosk; it is a kiosk's stack plus
more. If `/system/ext` can only hold unrelated, independent additions, this path
does not work and the mechanism needs revisiting before the graphics stack is
the thing that discovers it. §3's separate-mounts design is compatible with
layering, but layering has not been designed, and §7 now carries it.

**The licence boundary lands in the same place as the technical one**, which is
the strongest argument for the ruling and was not the reason it was made.
Checked 2026-08-22 against each project's own repository metadata:

| Component | Licence | Which path |
| :-- | :-- | :-- |
| `smithay` | MIT | kiosk |
| `iced` | MIT | kiosk |
| `libcosmic` | MPL-2.0 | desktop |
| **`cosmic-comp`** | **GPL-3.0** | desktop |

The kiosk path is pure MIT and sits comfortably inside `ADR-010` §4's allowlist.
The desktop path is **GPL-3.0**, and "steal from COSMIC extensively" means
*deriving from* `cosmic-comp` — which makes the derivative GPL-3.0 too.

Confined to a separately-distributed extension, that is clean: the base image
stays `MIT OR Apache-2.0` and the copyleft travels with the component that
carries it. Baked into a GUI *image variant*, it would have relicensed a
substantial part of Rivet's shipped system. **`ADR-010` §6.1's source-archive
machinery therefore has a second consumer**, and it is a more demanding one than
Buildroot's C mechanisms, because a GPL-3.0 derivative of Rivet's own code is
a different obligation from redistributing somebody else's unmodified GPL
binary.

---

## 7. Open

1. **Which extensions exist first.** `helix` is settled as the motivating case.
   A toolchain, debugging tools and a POSIX environment (`ADR-005` §5.4's
   deferred "official POSIX environment" — which is now clearly an extension
   rather than an image variant) are candidates. None is urgent.
2. **Whether extensions may be updated independently of the base image's A/B
   cycle**, or only alongside it. *Recommendation: independently.* That is the
   whole point of the mechanism, and it makes the rollback story more
   complicated in a way that needs thinking about before it is built rather
   than after.
3. **Extensions do not layer — RULED 2026-08-22. Each extension is a
   self-contained image.** A desktop extension contains its own compositor
   rather than depending on a kiosk one. There are **no dependency
   declarations, no resolution, and no mount ordering to get right.**

   The duplication is a few megabytes on a device whose card is 128 GB, and it
   is the same trade the project already takes elsewhere: static linking over
   shared libraries (`ADR-014` §4.2), `ext4` over `f2fs` (`ADR-018` §7).
   Layering is what you reach for when storage is scarce, and it is not.

   **This is load-bearing, not merely convenient.** `ADR-015` §3 exists to avoid
   building dependency resolution, and §6.1's "desktop builds on kiosk" was the
   first thing in the record that looked like it required one. It does not, so
   §3's refusal stands undisturbed and `ADR-015` §10 item 2's recommendation
   ("no, initially" — an application may not declare a dependency on an
   extension) is reinforced rather than contradicted.

   **What to watch.** The pressure to reverse this will not arrive as "let's add
   dependency resolution". It will arrive as a third extension that shares most
   of a second one, and a reasonable-sounding wish not to ship the overlap
   twice. The answer at that point is still no, and the reason is that the cost
   being avoided is disk space, which this project does not pay for.
4. **Where extension images are stored.** `ADR-018` §6 loop-mounts them from
   image files, so they live on `/state` — the only writable filesystem in
   `ADR-018` §4's layout. That section already sized `/state` as the remainder
   of the device with an adjustable reserved-space guarantee, so this is a
   placement question with an obvious answer rather than a sizing problem. An
   earlier draft of this item claimed otherwise; see `ADR-018` §9 item 2.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §7 item 3 — whether extensions layer was open

**Ruled 2026-08-22: no.** The superseded question was:

~~Do extensions layer?~~
