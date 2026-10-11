#!/usr/bin/env python3
"""Verify a generated mark against the stroked shape it is meant to represent.

`make_mark.py` emits the mark as *outlined filled paths* — annular sectors with
round caps, discs, polygons — because `build-brand.py` recolours a single fill
and would flatten strokes. That conversion is done by hand-rolled arc maths, so
it needs checking: a wrong large-arc or sweep flag produces a plausible-looking
but wrong shape, and the error is invisible at any size a human reviews at.

This reproduces the check numerically rather than by eye. For every point along
the emitted path's boundary it measures the distance to the boundary of the
shape a stroked construction *would* have produced. Zero everywhere means the
outline is exact. It also cross-checks enclosed area per subpath against the
analytic area, which catches a subpath that is the right shape in the wrong
place.

    uv run --with svgelements python verify_mark.py

Written because an earlier version of this check lived in a scratch file and its
"0.019 px" figure could not be reproduced from anything in the folder. Every
number quoted in BRAND.html or HANDOFF.md should trace to a script like this.

────────────────────────────────────────────────────────────────────────────
WHAT TO REWRITE FOR A DIFFERENT MARK

Generic, keep as-is:   sample_subpaths(), report(), main()
Project-specific:      geom(), deviation(), analytic_area(), and the CASES list

`deviation(x, y, ...)` answers one question: how far is this point from the
boundary of the intended shape? Express it as the minimum over every edge of the
construction. `analytic_area(...)` is the closed-form area of the same shape.
Both work in the design grid, not the emitted grid — the driver divides out
make_mark.SCALE before calling them.

Only applies when the mark is generated. A hand-drawn mark has no analytic
construction to check against; verify those visually at 1:1 instead.
────────────────────────────────────────────────────────────────────────────
"""

import math
import re
import sys

from svgelements import Path

import make_mark as mm

SAMPLES = 3000

# Thresholds. Deviation is judged at 32 px because that is the largest size the
# small asset has to serve; anything invisible there is invisible everywhere.
MAX_PX_AT_32 = 0.05
MAX_AREA_ERR = 0.005


# ── PROJECT-SPECIFIC ────────────────────────────────────────────────────────
# Rivet's mark is two rounded hexagons and a disc, read under `evenodd`. The
# useful identity is that a regular hexagon with rounded corners is exactly the
# MINKOWSKI SUM of a smaller regular hexagon with a disc of the corner radius:
#
#     rounded(R, c)  ==  hexagon(R - c/sin60)  (+)  disc(c)
#
# so the distance from any point to the rounded hexagon's boundary is the
# distance to that inset hexagon, minus c, in absolute value -- and its area is
# the inset hexagon's area, plus its perimeter times c, plus pi c squared. Both
# below follow from that one fact, which is why neither needs arc maths of its
# own to check the arc maths with.


def geom(r_out, band, r_core, corner, rot):
    """The intended construction, in make_mark's 100-unit design grid.

    Returns the two rounded hexagons as (inset circumradius, corner radius,
    rotation) triples, plus the core radius. The inner hexagon is derived from
    the outer APOTHEM exactly as make_mark derives it, so a change to that rule
    fails here rather than passing silently.
    """
    sin60 = math.sin(math.radians(60.0))
    apo_in = r_out * mm.COS30 - band
    r_in = apo_in / mm.COS30
    c_in = corner * r_in / r_out
    return (
        (r_out - corner / sin60, corner, rot),
        (r_in - c_in / sin60, c_in, rot),
        r_core,
    )


def _hex_verts(r, rot):
    return [(mm.CX + r * math.cos(math.radians(rot + k * 60.0)),
             mm.CY + r * sin_deg(rot + k * 60.0)) for k in range(6)]


def sin_deg(d):
    return math.sin(math.radians(d))


def _dist_to_hexagon(x, y, r, rot):
    """Unsigned distance from (x, y) to the boundary of a regular hexagon.

    The polygon is convex, so from any point -- inside or out -- the nearest
    boundary point lies on one of the six edge segments.
    """
    verts = _hex_verts(r, rot)
    best = float("inf")
    for k in range(6):
        ax, ay = verts[k]
        bx, by = verts[(k + 1) % 6]
        dx, dy = bx - ax, by - ay
        t = ((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)
        t = max(0.0, min(1.0, t))
        best = min(best, math.hypot(x - (ax + t * dx), y - (ay + t * dy)))
    return best


def deviation(x, y, outer, inner, core):
    """Distance from (x, y) to the nearest boundary of the intended shape."""
    best = abs(math.hypot(x - mm.CX, y - mm.CY) - core)
    for r_i, c, rot in (outer, inner):
        best = min(best, abs(_dist_to_hexagon(x, y, r_i, rot) - c))
    return best


def analytic_area(outer, inner, core):
    """Closed-form area of the same shape, in design units squared.

    Per subpath, summed the way sample_subpaths sums |shoelace| -- so the inner
    hexagon counts POSITIVELY here even though `evenodd` renders it as a hole.
    """
    total = math.pi * core ** 2
    for r_i, c, _rot in (outer, inner):
        total += (3 * math.sqrt(3) / 2) * r_i ** 2      # inset hexagon
        total += 6 * r_i * c                            # perimeter x corner
        total += math.pi * c ** 2                       # the six corner arcs
    return total


# Each entry: (svg file, label, kwargs for geom()). The small variant is a
# genuinely different drawing -- fewer units of border and core so its field
# survives small_strk -- so it gets its own parameters. Verify both.
CASES = [
    ("rivet-mark-rust.svg",  "standard mark", dict(mm.STANDARD)),
    ("rivet-mark-small.svg", "small variant", dict(mm.SMALL)),
]


# ── GENERIC DRIVER ──────────────────────────────────────────────────────────

def sample_subpaths(svg_path):
    """Split the single emitted path into subpaths and sample each one.

    Returns (points_per_subpath, total_shoelace_area, ink_width) in EMITTED
    units. Splitting on 'M' works because make_mark emits one moveto per closed
    subpath; a mark using relative 'm' would need adjusting here.
    """
    d = re.search(r'\sd="([^"]+)"', open(svg_path, encoding="utf-8").read()).group(1)
    subs = ["M" + s for s in d.split("M") if s.strip()]
    per_sub, area = [], 0.0
    xs = []
    for sub in subs:
        p = Path(sub)
        pts = [p.point(k / SAMPLES) for k in range(SAMPLES)]
        area += abs(sum(pts[k].x * pts[(k + 1) % SAMPLES].y
                        - pts[(k + 1) % SAMPLES].x * pts[k].y
                        for k in range(SAMPLES))) / 2
        per_sub.append(pts)
        xs += [pt.x for pt in pts]
    return per_sub, area, max(xs) - min(xs)


def report(svg_path, label, **kw):
    outer, inner, core = geom(**kw)
    S = mm.SCALE
    per_sub, area, ink_emitted = sample_subpaths(svg_path)

    worst, worst_at = 0.0, None
    for pts in per_sub:
        for pt in pts:
            # back into design units before comparing
            dv = deviation(pt.x / S, pt.y / S, outer, inner, core)
            if dv > worst:
                worst, worst_at = dv, (pt.x / S, pt.y / S)

    expect = analytic_area(outer, inner, core) * S * S
    ink = ink_emitted / S              # mark ink width, design units
    frac = worst / ink                 # deviation as a fraction of the mark
    area_err = abs(area - expect) / expect

    print(f"{label}  ({svg_path}, emitted at {S:g}x)")
    print(f"  subpaths sampled            {len(per_sub)} x {SAMPLES} points")
    print(f"  ink width                   {ink_emitted:.1f} emitted units"
          f"  ({ink:.1f} design)")
    if not 400.0 <= ink_emitted <= 800.0:
        print(f"    WARNING: outside the 400-800 unit band the family uses; "
              f"small_strk=20 would be {20 / ink_emitted * 100:.1f}% of the mark")
    print(f"  max boundary deviation      {worst:.6f} design units"
          f"  = {frac * 100:.4f}% of mark width")
    print(f"    -> at  16 px tile (88% fill) {frac * 16 * 0.88:.4f} px")
    print(f"    -> at  32 px tile            {frac * 32 * 0.88:.4f} px")
    print(f"    -> at 512 px tile            {frac * 512 * 0.88:.3f} px")
    print(f"    worst point (design units)  ({worst_at[0]:.2f}, {worst_at[1]:.2f})")
    print(f"  enclosed area               {area:.1f} vs analytic {expect:.1f}"
          f"   ({area_err * 100:.4f}% — chord sampling"
          f" under-measures arcs slightly)")
    ok = frac * 32 * 0.88 < MAX_PX_AT_32 and area_err < MAX_AREA_ERR
    print(f"  VERDICT: {'PASS — outline is geometrically exact' if ok else 'FAIL'}\n")
    return ok


def main():
    results = [report(f, label, **kw) for f, label, kw in CASES]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
