# Rivet brand — handoff

Open **[`BRAND.html`](BRAND.html)** first; it is the specification of record and
carries every asset in context, the measured contrast table and the reasoning.
This file is the version you can read without a browser.

---

## Status in one line

**Locked:** the accent `#B0512F`, the mark's geometry, and all nine generated
assets. **Not done:** nothing in the repository uses them yet — no README header,
no favicon, and the splash artwork is untouched.

---

## Source of truth

**The mark is GENERATED.** `make_mark.py` is the origin; every `rivet-mark-*.svg`
beside it is *output*.

> Editing path data in an SVG edits the wrong file. The next
> `uv run python make_mark.py` silently discards it, and because the assets are
> deterministic the loss will not show up as a diff conflict — it will just be
> gone. Change the geometry in `make_mark.py` and regenerate.

`brand.json` is the origin for the colours and the lockup configuration.
`BRAND.html` is written by hand and must be updated in the same pass as any asset
change — a stale handoff claiming a rejected mark was canonical has already
misdirected one session in this family.

---

## Generated assets

| File | What it is | Use it for |
|---|---|---|
| `logo-light.svg` | stacked lockup, light | documents, light UI |
| `logo-dark.svg` | stacked lockup, dark | dark UI, splash surfaces |
| `logo-h-light.svg` | horizontal lockup, light | README header, wide chrome |
| `logo-h-dark.svg` | horizontal lockup, dark | README header, dark |
| `appicon-light.svg` | brand icon, **dark tile** | light taskbars and docks |
| `appicon-dark.svg` | brand icon, **lifted tile** | dark taskbars and docks |
| `appicon-mono-light.svg` | monochrome icon | surfaces that forbid colour |
| `appicon-mono-dark.svg` | monochrome icon, reversed | as above, dark |
| `appicon-small.svg` | white mark, flat tile, emboldened | **≤32 px only** — `.ico`, `.icns`, favicons |

The `-light` / `-dark` suffix names **the chrome the icon sits on**, not the tile
colour. `appicon-light.svg` is the *dark* tile, because that is what pops against
a light taskbar.

---

## Lockup geometry

Measured at build, not asserted:

| Quantity | Value |
|---|---|
| mark ink | 467.7 × 530.7 (aspect 0.8812) |
| measure `W` | 708.1 — derived from the wordmark's natural width |
| cap height | 170.2 (0.1702 × W) — **the family constant** |
| descriptor | 47.94 @ 0.326em tracking |
| icon fill | 67% · small fill 88% |

`wordmark_width` is deliberately **omitted**. Only Athena pins it, to preserve
legacy letter-spacing; a new project derives the measure. And the constant held
across the family is **cap height, never lockup width** — forcing a fixed measure
squeezed `OLYMPUS` by −0.11em.

---

## Brand colours

**Rivet's colour is `#B0512F`** — weathered oxide, HSL hue 15.8°, ruled
2026-09-01.

| Key | Value | Role |
|---|---|---|
| `accent_on_light` | `#B0512F` | the mark on every light lockup — **the ruled colour** |
| `accent` | `#C06139` | mark + descriptor on dark, and the mark on the dark tile |
| `accent_bright` | `#D07A4E` | the mark on the lifted dark-chrome tile only |
| `accent_deep` | `#8F3F22` | the descriptor on light |
| `ink` | `#2B3340` | wordmark on light — **family constant** |
| `paper` | `#F4F6F8` | wordmark on dark — **family constant** |

Measured with `check_color.py`:

| Colour | white | paper | dark chrome `#111` | tile `#20120D` |
|---|---|---|---|---|
| `#B0512F` | 5.17 | 4.77 | 3.65 | 3.52 |
| `#C06139` | — | — | **4.51** | **4.34** |
| `#D07A4E` | — | — | — | **5.04** on `#2E1D16` |
| `#8F3F22` | **7.24** | 6.68 | — | — |
| `#FFFFFF` | — | — | — | 18.20 |

**Why the accent has two values.** `#B0512F` is light-biased and dims on black —
3.65 on dark chrome is the family's weakest dark reading, below the 3.84 that
retired Trends' `#12805E`. Left as the single accent it would put that number on
the dark lockup's *descriptor*, which is small text rather than a graphic. So
`accent` carries a small lift and `accent_on_light` holds the ruled colour. This
is the Olympus mechanism inverted — there the accent vanished on white, here it
dims on black.

**Tiles are dark on purpose.** A warm charcoal `#332E2C` was tried first and read
**2.59:1** against the mark — a failure of 3:1 outright.

### The Ignition proximity was ruled, not missed

`#B0512F` sits **11.8° from Ignition's `#EF7C1A`** — closer than any accent this
family has ever rejected (the three killed in one pass sat at 7°, 8° and 10°) and
well inside the 32° the accepted jade held. It clears `check_color.py`'s 12° flag
by two tenths of a degree.

The numbers were put to the maintainer before the colour was chosen and he ruled
it: he did not care about crowding Ignition's color, because the two projects
will live side by side (`.rulings/` R-294). **Do not re-open this as
an oversight.** The supporting argument is the one the family already accepts for
Ignition and Athena, which sit 9.1° apart and coexist:

| | Hue | Lightness | Saturation |
|---|---|---|---|
| Rivet | 15.8° | 43.7% | 57.8% |
| Ignition | 27.6° | 52.0% | 86.9% |

A muted dark corroded red-brown against a vivid flame orange — 29.1 points of
saturation apart. **Still unverified:** whether the two marks stay distinct side
by side at 16 px.

---

## Small sizes

`appicon-small.svg` is **a different drawing, not a downscale**, and the way it
differs is counter-intuitive.

It was first drawn *thicker* than the standard mark — border 13, rivet r 17 — on
the instinct that small sizes want more weight. Rasterised at true 16 px, the
centre row read:

```
before   .+##+######+##+.    field gone — border welded to the rivet
after    .+##.+####+.##+.    field survives as a full dark pixel
```

`small_strk` fattens every edge at once, so the field closes from **both** sides.
The fix was to give weight *back* — border 12, rivet r 15 — so the field starts
wider: 11.97 design units before embolden, 8.64 after, or 1.35 px at 16 px.

**Nothing errors when this is wrong.** The icon just renders as a lozenge, and a
preview pane that scales will not show it. Re-check by rasterising at true 16 px
if these numbers are ever touched.

---

## Regenerating

From `docs/brand/`, in order:

```bash
uv run python make_mark.py
```

```bash
uv run --with svgelements python verify_mark.py
```

```bash
uv run --with uharfbuzz --with fonttools --with svgelements python build-brand.py ./brand.json
```

Deterministic — same inputs give byte-identical output, so it is safe to run in
CI as a drift check.

`verify_mark.py` samples each subpath against the intended construction and
cross-checks enclosed area. Current result: **max boundary deviation 0.000042
design units** (0.0001% of the mark's width), area 312 373.1 against an analytic
312 373.3. Hand-rolled arc maths produces plausible-looking but wrong shapes when
a sweep flag is wrong, and the error is invisible at any size a human reviews at
— which is why this is a script and not an eyeball.

---

## What is vendored here

`docs/brand/` regenerates standalone. It holds `make_mark.py`, `verify_mark.py`,
`build-brand.py`, `Geist-var.ttf` and `OFL.txt`. `build-brand.py` looks for the
font beside itself first, so the side-by-side copy works with no edits.

**`OFL.txt` is not optional.** The SIL OFL requires the licence to travel with
the font, and it is an explicit acceptance criterion of Athena's adoption ticket
`39ac7a` — every adoption in this family up to and including Trends missed it.
Its copyright line reads *Copyright 2024 The Geist Project Authors*, matching
`Geist-var.ttf`'s own name table.

---

## Integration

README header, theme-aware:

```html
<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-h-dark.svg">
    <img src="docs/brand/logo-h-light.svg" alt="Rivet — appliance os" width="420">
  </picture>
</p>
```

Favicons and launcher icons — `appicon-small.svg` is the single source:

```bash
magick -background none docs/brand/appicon-small.svg -resize 32x32 favicon-32.png
```

```bash
magick docs/brand/appicon-small.svg -define icon:auto-resize=32,24,16 favicon.ico
```

**Rivet is not Athena-managed**, so there is no adoption ticket. The decision
record is [`../ADR-037-Brand-Identity.md`](../ADR-037-Brand-Identity.md), and it
is the document to amend if any of this changes.

---

## Licence and repository

The assets are part of Rivet and carry the repository's MIT licence
(`LICENSE.txt`, Joel Lindell). `Geist-var.ttf` is separately licensed under SIL
OFL 1.1 — see `OFL.txt` in this folder.

---

## Still open

- **Nothing in the repository uses these assets.** No README header, no favicon.
  Adoption is a separate change.
- **The splash and launcher artwork is untouched.** `assets/shield-512.rgba` is
  still the two-tone steel shield, shared by `cmd/rivet-splash` and
  `cmd/rivet-launcher`. Whether the boot screen should show the hexagon instead
  is an open question, and it would mean regenerating a 1 MiB raw asset that two
  programs draw. The shield and the mark are relatives, not versions of each
  other.
- **The Ignition proximity is unverified in use** — measured, but not seen side by
  side at small sizes.
- **The fleet TUI's red has not been re-checked against the brand.**
  `cmd/rivet-fleet/src/tui/theme.rs` makes `down()` and `error()` `Color::Red`,
  and `cmd/rivet-provision/src/field.rs` rules that *red at write time is a
  rejection*. Those are semantic and must not be reassigned to decoration. If the
  fault state stops being unmistakable beside brand chrome, move the **semantic**
  colour, not the accent.
- **`#B0512F` is not yet in the brand-kit skill's family table** or in
  `check_color.py`'s `FAMILY` dict. A project missing from that dict cannot be
  collided with — the omission has already produced one wrong answer (Reliquary)
  and one six-day gap for another sibling. That file lives in `~/.claude/` and is mirrored
  to `caladil/claude-environment`; it needs an export and a push.
