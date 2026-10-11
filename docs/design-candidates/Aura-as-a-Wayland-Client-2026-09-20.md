# Aura as a Wayland client, and where the seam goes

* **Status:** **RULED — Option A**, 2026-09-20. Asked which of three seams;
  the maintainer chose A (`.rulings/` R-318). The Wayland backend goes in `rivet-sdk`
  behind a feature, and an application changes one call.
* **Where it is implemented:** `ADR-058` §5.4 carries the decision; this
  document stays as the options that were weighed and the reason A won.
* **Date:** 2026-09-20
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-058` §4.1 landed on 2026-09-20: `rivet-displayd` owns the screen and hands
out surfaces, and `rivet-launcher` is its first client. **Every other kiosk
application on this machine is now broken in principle**, `aura-renderer`
included — it declares `devices = ["display"]` and takes a DRM lease, and a
compositor cuts no leases.

The maintainer asked the question before the compositor existed: whether Aura
also becomes a Wayland client in the long run (`.rulings/` R-318). The answer is
yes and this is what it costs.

**Nothing in Aura has been changed.** `aura-renderer` still runs on
`rivet-01` under the old model and is mid-soak there.

---

## 2. What Aura actually uses

`rivet_sdk::screen::Screen` is **five items wide**, which is the whole reason
this is tractable:

| | |
| :-- | :-- |
| `width`, `height`, `stride` | where the pixels go |
| `what` | the sentence the program prints about its display |
| `blit(&canvas)` | hand over a finished frame |

Everything else — mode enumeration, `drmModeSetCrtc`, double buffering, page
flips, the missed-event policy — is **behind** that. `aura-renderer` never
names any of it.

**So the drawing code does not change at all.** Aura composes a frame into a
`&[u8]` and calls `blit`. A Wayland implementation of those five items is a
different `Screen` and the same application.

---

## 3. The question: where does the Wayland go?

### Option A — a `composited` feature on `rivet-sdk`

`take_composited()` returns the same `Screen` shape, backed by
`smithay-client-toolkit` against the socket `rivet_core::compositor` names.

* **For:** `ADR-036` §5.4A is *an application is handed what it needs*, and the
  SDK is where that promise is kept. Aura changes one call. So does every
  future kiosk application, and none of them learns Wayland.
* **For:** the precedent exists — `screen = ["draw"]` is already an opt-in
  feature that links `libdrm`, added for exactly this reason (*"a network
  service has no business linking libdrm"*). A `composited` feature is that
  argument again with a different library.
* **For:** it stays STATIC. `cmd/rivet-launcher` proved it — SCTK with
  `default-features = false` is pure Rust, 0 `NEEDED` entries.
* **Against:** `rivet-sdk`'s `[dependencies]` table is empty **by rule**, and
  this adds a third-party crate to the library every application links. The
  rule already has the `screen`/`serve` exceptions, so this is a third rather
  than a first — which is the thing to argue about rather than wave through.

### Option B — Aura speaks Wayland itself

`aura-renderer` depends on `smithay-client-toolkit` directly and keeps its own
surface, as `cmd/rivet-launcher` does.

* **For:** no change to the SDK, and no new dependency for anything that does
  not draw.
* **Against:** it makes every kiosk application learn Wayland — which is
  `ADR-058` §3.1's complaint about the LEASE, arriving one layer up. The whole
  argument for a compositor was that an application should be handed a result
  rather than a mechanism; handing it a Wayland connection instead of a DRM
  lease is a smaller version of the same mistake.
* **Against:** N implementations of surface handling, `ADR-011`.

### Option C — both, in order

Do B first in Aura to find out what the shape wants to be, then hoist to A once
two applications have needed it.

* **For:** `converge-first-parameterize-second` is the maintainer's own rule
  for exactly this, and `rivet-launcher` is already the first implementation —
  so Aura would be the **second**, which is when a shape is usually known.
* **Against:** it puts a Wayland connection in an example application, which is
  what every future author will copy.

---

## 4. The recommendation

**Option A**, and `rivet-launcher` is the reason rather than the theory: it is
already the second implementation of this, after the SDK's DRM one, and its
`surface.rs` is about 200 lines of which none is Aura-specific. The shape is
known. Hoisting it into the SDK now means Aura's change is one call rather
than a module.

The dependency objection is real and is answered by the existing precedent
rather than dismissed: a feature nothing enables by default costs a network
service nothing, which is the exact bargain `screen = ["draw"]` already struck.

---

## 5. What else has to move, and it is not all drawing

* **`devices = ["display"]` comes out of Aura's manifest.** That is the whole
  of `ADR-058` §3.2 — no DRM node, no render node, none of §1.1's five grants.
* **`connects = ["rivet-displayd:wayland"]` goes in**, which is what makes a
  missing compositor a named refusal rather than a black screen.
* **`rivet_sdk::screen::Handed::capture` is dead** under a compositor. `ADR-058`
  §5.3 moved capture to `rivet-displayd`, which is the one program that can see
  the screen; an application photographing its own frame was always the
  narrower thing and is now the wrong one.
* **`Screen::what`'s sentence changes**, and should: a program that prints
  *"card1, HDMI-A-2, double-buffered"* would be describing a machine it no
  longer talks to.
* **The addon frame boundary is untouched**, which is worth stating because it
  looks like it should move. `aura-background-pack` draws into what its host
  hands it; the host's surface changing owner does not move that seam.

---

## 6. What this does NOT decide

**Whether Aura gets a GPU surface.** `GPU-Backgrounds-2026-09-20` §5 asked
whether a pack could render on the GPU, and under Wayland that question becomes
*does Aura use `wl_shm` or `linux-dmabuf`* — a better version of the same
question, because both answers already exist and neither has to be invented.
`wl_shm` is what this document proposes, because it is what Aura does today.
