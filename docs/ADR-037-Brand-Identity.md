# ADR-037: Brand Identity

* **Status:** Accepted — ruled and executed 2026-09-01. Descriptor corrected
  2026-09-05, see §2.
* **Date:** 2026-09-01
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Supersedes nothing.** It gives Rivet a visual identity it did not have, and
  records one decision that will otherwise be mistaken for an oversight (§3).
* **Maintainer rulings, 2026-09-01** *(`.rulings/` R-294)*:
  * The concept: the Rust Rivet is a red-colored hexagon border with a figure
    inside it.
  * The composition: a thick-bordered hexagon, a bit of white field, a circle.
  * Two rejections and the orientation: the first colour was not rusty enough
    and was too red, and the hexagon points up.
  * §3: crowding Ignition's color does not matter to him, because the two
    projects will live side by side.
  * The descriptor, as first ruled, was `AGENTIC PLATFORM`; §2 records its
    correction.

---

## 1. Context

Rivet had artwork but no identity. `assets/shield-512.rgba` — a two-tone steel
shield with an oxide rivet at its centre, drawn by `cmd/rivet-splash` and
`cmd/rivet-launcher` — was the only mark the project owned, and it exists to be
blitted to a framebuffer, not to sit above a wordmark or inside a 16 px tile.

The maintainer's other Rust projects — Athena, Olympus, Ignition, Tachyon,
Trends, Lodestone and Reliquary — share a documented
identity system: one accent colour per project, Geist as the typeface for all of
them, and a common set of nine generated assets. Rivet joins it.

The assets, the specification of record and the terse handoff live in
`docs/brand/`. This ADR records only what was **decided**, and it is the document
to amend if any of it changes.

---

## 2. The decision

**Rivet's colour is `#B0512F`** — weathered oxide, HSL hue 15.8°, lightness
43.7%, saturation 57.8%.

**Rivet's mark is a pointy-top hexagonal plate with a thick border, a field, and
a rivet at its centre**, drawn as one closed filled path in a single colour.

**Rivet's descriptor is `APPLIANCE OS`.** *(Corrected 2026-09-05 — ruled here
2026-09-01 as `AGENTIC PLATFORM`, which named how Rivet is operated rather than
what it is.)*

### 2.1 The mark is generated, not hand-drawn

Both origins are legitimate in this family — Athena's owl and Tachyon's glyphs
are hand-drawn and their committed SVGs are canonical; Trends' and Reliquary's
are script output. Rivet's is **generated**.

`docs/brand/make_mark.py` is the source of truth. Every `rivet-mark-*.svg` beside
it is output, and editing path data in one of them edits the wrong file: the next
run discards it silently, and because the build is deterministic the loss will
not surface as a diff conflict.

### 2.2 Pointy-top, and why the orientation is not arbitrary

A **flat-top** hexagon with a circle in the middle is the exact silhouette of a
hex **nut** — a different fastener from a rivet, and a misread the project's own
name invites. Pointy-top also stands taller than wide (77.9 × 88.4 design units),
which centres better in a square icon tile and above the wordmark.

### 2.3 The splash artwork is not superseded

`assets/shield-512.rgba` stays exactly as it is. It **cannot** be the brand mark:
the generator recolours one fill and would flatten the shield's second colour.
That is the same constraint that stopped Ignition's charcoal-hexagon-plus-
gradient-flame concept being taken as-is.

The hexagon is the single-colour reduction of the shield's idea, not a
replacement for it. **The two are relatives, not versions of each other** —
changing one does not oblige a change to the other. Whether Rivet's boot screen
should show the hexagon is an open question this ADR does not answer, and it
would mean regenerating a 1 MiB raw asset that two programs draw.

---

## 3. **The Ignition collision was ruled, not missed**

**Read this before filing a defect about Rivet's accent.**

`#B0512F` sits **11.8° from Ignition's `#EF7C1A`**. That is closer than any
accent this family has ever rejected — the three killed in one pass for Trends
sat at 7°, 8° and 10° — and far closer than the 32° the accepted jade held. It
clears the brand-kit's 12° advisory flag by two tenths of a degree.

**This was known before the colour was chosen.** The full separation table was
put to the maintainer, with the rejected precedents named, and he ruled that he
did not care about crowding Ignition's colour, because the two projects will live
side by side. *(`.rulings/` R-294.)*

The reasoning that supports it is the one the family already accepts for Ignition
and Athena, which sit **9.1°** apart and coexist perfectly well:

| | Hue | Lightness | Saturation |
| :-- | :-- | :-- | :-- |
| Rivet `#B0512F` | 15.8° | 43.7% | 57.8% |
| Ignition `#EF7C1A` | 27.6° | 52.0% | 86.9% |

Hue separation alone is not the test. A muted, dark, corroded red-brown and a
vivid flame orange are **29.1 saturation points** apart, which is what does the
separating here.

**What remains unverified:** whether the two marks stay distinct *side by side at
16 px*. The lightness and saturation figures are measured; that comparison has
not been made.

### 3.1 Two reds were rejected on the way

| Candidate | Hue | From Ignition | Outcome |
| :-- | :-- | :-- | :-- |
| `#D63A33` hot iron | 2.6° | 25.0° | rejected as too red |
| `#DE3B4B` scarlet | 358.5° | 29.1° | not taken — least "Rust" of the set |
| `#B7410E` true rust | 18.1° | **9.5°** | inside every rejected precedent |
| **`#B0512F` weathered oxide** | **15.8°** | **11.8°** | **ruled** |

The safest reds on hue were the least rusty, and the rustiest were the least
safe. `#B0512F` is the point the maintainer settled on.

---

## 4. The accent has two values, and that is deliberate

`#B0512F` is **light-biased**: 5.17:1 on white and 4.77:1 on paper, but **3.65:1
on dark chrome** — the family's weakest dark reading, below the 3.84 that retired
an earlier Trends jade for that reason alone.

The generator uses `accent` for the mark **and the descriptor** on dark lockups.
The descriptor is small tracked-out text, not a graphic, so 3.65:1 is not
acceptable there. Therefore:

| Key | Value | Role |
| :-- | :-- | :-- |
| `accent_on_light` | `#B0512F` | the mark on every light lockup — **the ruled colour** |
| `accent` | `#C06139` | mark + descriptor on dark; the mark on the dark tile |
| `accent_bright` | `#D07A4E` | the mark on the lifted dark-chrome tile only |
| `accent_deep` | `#8F3F22` | the descriptor on light |

This is the **Olympus mechanism inverted**. There the accent vanished on white
(the retired `#7BEEFF` read ~1.35:1) and the deep twin was for light; here it
dims on black and the lifted twin is for dark.

**`accent` in `brand.json` is a role, not the identity.** Rivet's colour is
`#B0512F`, and that is the value that belongs in any cross-project comparison.

---

## 5. Brand colour is not UI semantics, and Rivet already spends red

**This is the one place the identity touches running code.**

* `cmd/rivet-fleet/src/tui/theme.rs` makes `down()` and `error()` `Color::Red`.
* `cmd/rivet-provision/src/field.rs` carries a ruling built entirely on it:
  *"red at write time is a rejection, red already on the screen is a state you
  can see and fix."*

Those are **semantic** colours. They carry meaning and **must not be reassigned
to decoration** because the brand now occupies adjacent ground.

This is the same collision that pushed Trends off a magenta accent — a sibling
application had to reserve red for emergency semantics. The resolution differs:
there the brand moved, here it is staying, because Rivet's fault states live in a
**terminal palette this project does not control** and are separated from the
oxide mostly by saturation.

**Unverified:** whether the fleet TUI's fault state remains unmistakable beside
brand chrome, on a real terminal, at a real size. **If it fails, move the
semantic colour, not the accent.**

---

## 6. Constraints that must be recorded, not just followed

These are the things most likely to be silently undone by someone tidying up.
Each one has a failure behind it, and none of them announces itself when broken.

1. **One closed filled path, one colour per file.** The generator recolours a
   single fill and flattens anything else. No `stroke`, no `<circle>`, no
   gradients.
2. **`fill-rule` is `evenodd` and it is load-bearing.** Under `nonzero` the field
   fills in and the mark becomes a solid slab with an invisible dot on it. It is
   not obvious at thumbnail size. Never normalise it against the wordmark's
   `nonzero` — doing exactly that punched white notches into Athena's `H`, and it
   shipped that way.
3. **The mark is emitted at 6× a 100-unit design grid.** `small_strk = 20` is a
   literal stroke width in the mark file's own units, not a fraction of the mark.
   At 1× it would be 20% of the drawing. Ink is currently 467.7 units, inside the
   family's 400–800 band.
4. **`appicon-small.svg` is a different drawing, not a downscale — and it is
   THINNER, not thicker.** It was first drawn heavier on the instinct that small
   sizes want weight. Rasterised at true 16 px the centre row read
   `.+##+######+##+.`: the field was gone and the border had welded to the rivet.
   `small_strk` fattens every edge at once, so the field closes from *both*
   sides. Giving weight back fixed it — it now reads `.+##.+####+.##+.`
   **Nothing errored.** Re-check by rasterising, never by eye in a scaling
   preview pane.
5. **No shipped asset may contain a `<text>` element or a `font-family`.** Every
   glyph is baked to outlines. Live text renders differently on every machine.
6. **`OFL.txt` must travel with `Geist-var.ttf`.** The SIL OFL requires it, and
   every adoption in this family up to and including Trends missed it.
7. **`wordmark_width` is deliberately omitted.** Only Athena pins it, for legacy
   letter-spacing. The family constant is **cap height, never lockup width** —
   forcing a fixed measure squeezed `OLYMPUS` by −0.11em.
8. **Tiles are dark on purpose.** A warm charcoal `#332E2C` was tried and read
   2.59:1 against the mark, failing 3:1 outright.

---

## 7. What was verified, and how

| Claim | Method | Result |
| :-- | :-- | :-- |
| Outline geometry is exact | `verify_mark.py`, 3 subpaths × 3000 points, against the analytic construction | max deviation **0.000042** design units; area 312 373.1 vs 312 373.3 |
| Ink inside the family band | `build-brand.py` at build | 467.7 units, no warning |
| Contrast on every ground | `check_color.py` | table in §4 and `BRAND.html` §05 |
| Hue separation vs every sibling | `check_color.py` | table in §3 |
| Small variant survives 16 px | rasterised to a 16×16 canvas in a real browser, centre row sampled | `.+##.+####+.##+.` — field survives |
| All nine assets on light, dark and mid grey | rendered at true 1:1 over HTTP in a real browser | no defects |
| No `<text>` / `font-family` in any shipped asset | grep over all nine | clean |

**Not verified, and not claimed:** nothing in the repository uses these assets
yet. No README header, no favicon, and the splash artwork is untouched. Adoption
is a separate change.

---

## 8. Consequences

* `docs/brand/` regenerates standalone — `make_mark.py`, `verify_mark.py`,
  `build-brand.py`, `Geist-var.ttf` and `OFL.txt` are vendored there. The build
  is deterministic, so it can be run in CI as a drift check.
* **`README.md` still describes Rivet as "A Rust-first Linux distribution for the
  Raspberry Pi."** Rivet's descriptor was corrected 2026-09-05 to `APPLIANCE OS`
  (see §2), which sits far closer to that description than `AGENTIC PLATFORM`
  did. Reconciling the two remains a documentation decision this ADR does not
  make.
* `#B0512F` must be added to the brand-kit skill's family table and to
  `check_color.py`'s `FAMILY` dict. **A project missing from that dict cannot be
  collided with** — the omission has already produced one wrong answer
  (Reliquary, told a session it was "comfortably clear" of a sibling 10.1° away)
  and one six-day gap for another sibling. That file lives outside this repository.
