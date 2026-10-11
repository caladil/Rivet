#!/usr/bin/env python3
"""Build Rivet's mark as outlined, filled SVG paths.

    uv run python make_mark.py

**The SVGs in this folder are OUTPUT.** This file is the source of truth for the
mark's geometry; editing path data in `rivet-mark-*.svg` edits the wrong file and
the next run silently discards it.

The concept comes from the splash artwork in `assets/shield-512.rgba` — a steel
plate with a single rivet at its centre. That drawing is two-tone (steel body,
oxide rivet) and stays that way where it lives, in `cmd/rivet-splash` and
`cmd/rivet-launcher`. It cannot be the brand mark: `build-brand.py` recolours ONE
fill and would flatten a second colour, which is the documented reason Ignition's
charcoal-hexagon-plus-gradient-flame concept could not be taken as-is either. So
this is the single-colour reduction of the same idea, to the maintainer's brief
of 2026-09-01: **a thick-bordered hexagon, a bit of field, and the rivet.**

Two hard constraints from the playbook shape everything below:

* **One closed filled path, one colour per file.** Every shape here is emitted
  as outlined geometry — never `stroke`, never `<circle>`, never two fills.
* **Ink 400-800 units wide.** See SCALE.
"""

from math import cos, sin, tan, radians

# -- PROJECT-SPECIFIC --------------------------------------------------------
PROJECT = "Rivet"
SLUG = "rivet"

COLOURS = {
    "rust":  "#b0512f",     # the accent, ruled 2026-09-01; HSL hue 15.8 deg
    "white": "#ffffff",     # small tiles and dark chrome
    "black": "#000000",     # single-colour print / mono lockups
}

# The mark is designed on a 100-unit grid and EMITTED at SCALE times that,
# because build-brand.py's small-size embolden (`small_strk = 20`) is a literal
# stroke width in raw mark path units, not a fraction of the mark. At 1x it
# would be 20% of this drawing and would weld the rivet to the rim; at 6x the
# hexagon spans 468 units across the flats and the embolden is 4.3%, against
# Athena's 479-unit owl at 4.17% -- which is what that default was tuned
# against. build-brand.py warns when the ink falls outside 400-800; do not
# shrink SCALE without re-checking appicon-small.svg at true 16 px.
#
# The failure is silent. Nothing errors; the icon just renders as a blob.
SCALE = 6.0

CX = CY = 50.0

COS30 = cos(radians(30.0))


# -- OUTLINING PRIMITIVES ----------------------------------------------------
# Two generic converters from a geometric construction to filled outlines: a
# disc, and a convex polygon with rounded corners. The whole mark is those two.


def disc(r, cx=None, cy=None):
    """A closed circle as two half-arcs.

    No `<circle>` element, so the whole mark stays one `<path>` the generator
    can recolour in a single substitution.
    """
    cx = CX if cx is None else cx
    cy = CY if cy is None else cy
    rs, x, y = r * SCALE, cx * SCALE, cy * SCALE
    return (
        f"M{x - rs:.3f} {y:.3f}"
        f"A{rs:.3f} {rs:.3f} 0 1 0 {x + rs:.3f} {y:.3f}"
        f"A{rs:.3f} {rs:.3f} 0 1 0 {x - rs:.3f} {y:.3f}Z"
    )


def rounded_hexagon(r, rot, corner):
    """A regular hexagon of circumradius `r`, rotated `rot` degrees, with its
    six corners rounded to radius `corner`.

    Emitted as one closed subpath: a straight run along each edge, then a corner
    arc. The cutback along each edge either side of a vertex is
    `corner / tan(interior/2)`; a hexagon's interior angle is 120 degrees, so
    that is `corner / tan(60)`.

    Vertices are visited in order of increasing angle. SVG's y axis increases
    downward, so that runs CLOCKWISE on screen, which is why every corner arc
    sweeps 1. `corner = 0` degenerates to a plain polygon and skips the arcs.
    """
    verts = [(CX + r * cos(radians(rot + k * 60.0)),
              CY + r * sin(radians(rot + k * 60.0))) for k in range(6)]

    if corner <= 0.0:
        d = f"M{verts[0][0] * SCALE:.3f} {verts[0][1] * SCALE:.3f}"
        for x, y in verts[1:]:
            d += f"L{x * SCALE:.3f} {y * SCALE:.3f}"
        return d + "Z"

    cut = corner / tan(radians(60.0))
    rs = corner * SCALE

    def unit(a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        n = (dx * dx + dy * dy) ** 0.5
        return dx / n, dy / n

    d = ""
    for k in range(6):
        prev, here, nxt = verts[(k - 1) % 6], verts[k], verts[(k + 1) % 6]
        ib = unit(here, prev)     # back along the incoming edge
        ob = unit(here, nxt)      # forward along the outgoing edge
        start = ((here[0] + ib[0] * cut) * SCALE, (here[1] + ib[1] * cut) * SCALE)
        end = ((here[0] + ob[0] * cut) * SCALE, (here[1] + ob[1] * cut) * SCALE)
        d += (f"M{start[0]:.3f} {start[1]:.3f}" if k == 0
              else f"L{start[0]:.3f} {start[1]:.3f}")
        d += f"A{rs:.3f} {rs:.3f} 0 0 1 {end[0]:.3f} {end[1]:.3f}"
    return d + "Z"


# -- PROJECT-SPECIFIC -- the composition -------------------------------------


def mark(r_out, band, r_core, corner, rot):
    """A thick-bordered hexagon with the rivet at its centre.

    Read under `fill-rule="evenodd"`, which is LOAD-BEARING here and must match
    `mark_fill_rule` in brand.json:

      * outer hexagon    parity 1  -> the plate
      * inner hexagon    parity 2  -> the border's inner edge, punched out
      * centre disc      parity 1  -> the rivet, sitting in the punched field

    Under `nonzero` the field fills in and the mark becomes a solid slab with an
    invisible dot on it. That is not obvious at thumbnail size, which is exactly
    why the key exists and why it must never be normalised against the
    wordmark's `nonzero`.

    The border is of uniform thickness, so the inner hexagon is derived from the
    outer one's APOTHEM, not its circumradius -- subtracting `band` from the
    circumradius would give a border thinner at the flats than at the corners.
    """
    apo_out = r_out * COS30
    apo_in = apo_out - band
    r_in = apo_in / COS30

    return (
        rounded_hexagon(r_out, rot, corner)
        + rounded_hexagon(r_in, rot, corner * r_in / r_out)
        + disc(r_core)
    )


def svg(d, colour, title, fill_rule="evenodd"):
    """One path, one fill, no `<text>`, no external reference."""
    s = int(100 * SCALE)
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}" '
        f'role="img" aria-label="{title}">'
        f"<title>{title}</title>"
        f'<path fill="{colour}" fill-rule="{fill_rule}" d="{d}"/>'
        "</svg>\n"
    )


# POINTY-TOP (rot 30), ruled 2026-09-01. A flat-top hexagon with a circle in it
# is the exact silhouette of a hex NUT, which is a different fastener from a
# rivet. Pointy-top also stands taller than wide, which centres better in a
# square icon tile and above the wordmark in the stacked lockup.
#
# Border 12, field 10, rivet 34 across -- the three bands are close enough in
# weight that none of them reads as trim, which is the whole point of the
# maintainer's brief: a thick-bordered hexagon, a bit of white field, a circle.
STANDARD = dict(r_out=45.0, band=12.0, r_core=17.0, corner=5.0, rot=30.0)

# Small variant. Small sizes need a DIFFERENT DRAWING, not a downscale, and the
# thing that has to change is not what intuition suggests: `small_strk` fattens
# every edge at once, so the field between the border and the rivet closes from
# BOTH sides and the border and rivet weld into a blob.
#
# The first attempt here was band 13 / core 17 -- thicker than the standard mark,
# on the reasoning that small sizes want more weight. Rasterised at true 16 px,
# its centre row read `.+##+######+##+.`: the field was a single half-lit pixel
# and the mark was a lozenge. Nothing errored, and it is invisible in a preview
# pane that scales. The fix is the opposite of the instinct -- give BACK weight
# so the field can start wider:
#
#   embolden eats 20 emitted units = 3.33 design units out of the field total
#   field before  = 45*cos30 - band - r_core = 38.97 - 12 - 15 = 11.97
#   field after   = 11.97 - 3.33 = 8.64 units = 1.35 px at 16 px  -> clears 1 px
#
# Re-check by rasterising, not by eye, if these numbers are ever touched.
SMALL = dict(r_out=45.0, band=12.0, r_core=15.0, corner=5.0, rot=30.0)


if __name__ == "__main__":
    std = mark(**STANDARD)
    for name, colour in COLOURS.items():
        open(f"{SLUG}-mark-{name}.svg", "w", encoding="utf-8").write(
            svg(std, colour, PROJECT)
        )
        print(f"wrote {SLUG}-mark-{name}.svg")
    open(f"{SLUG}-mark-small.svg", "w", encoding="utf-8").write(
        svg(mark(**SMALL), "#ffffff", PROJECT)
    )
    print(f"wrote {SLUG}-mark-small.svg")
    print("\nDeterministic: same inputs, byte-identical output.")
    print("Next: uv run --with svgelements python verify_mark.py")
