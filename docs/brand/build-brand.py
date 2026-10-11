"""Shared brand-asset generator for the project family.

Reads a `brand.json` next to a project's mark and emits the full asset set, with
the wordmark converted to real outlines so nothing depends on an installed font.

    uv run --with uharfbuzz --with fonttools --with svgelements \
        python <skill>/scripts/build-brand.py ./brand.json

Outputs, into the folder holding brand.json:
    logo-light.svg / logo-dark.svg          stacked lockup
    logo-h-light.svg / logo-h-dark.svg      horizontal lockup
    appicon-light.svg / appicon-dark.svg    brand app icon
    appicon-mono-light.svg / -dark.svg      monochrome app icon
    appicon-small.svg                       emboldened, for <=32 px

This is the generalised form of the script that produced Athena's assets; see
`reference/playbook.md` for the reasoning behind every constant.

Deterministic: the same inputs give byte-identical output, so it is safe to run
in CI as a drift check.
"""
import json, os, re, sys
import uharfbuzz as hb
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from svgelements import SVG, Shape, Path

KIT = os.path.dirname(os.path.abspath(__file__))
# The font is looked for beside this script first, then in the skill's assets/.
# That order matters: at adoption time `build-brand.py` and `Geist-var.ttf` are
# copied side by side into the project's `docs/brand/`, and the copy must keep
# working there with no reference back to the skill. Set BRAND_KIT_FONT to
# override.
FONT = next((p for p in (
    os.environ.get("BRAND_KIT_FONT", ""),
    os.path.join(KIT, "Geist-var.ttf"),
    os.path.join(KIT, os.pardir, "assets", "Geist-var.ttf"),
) if p and os.path.exists(p)), os.path.join(KIT, "Geist-var.ttf"))
if not os.path.exists(FONT):
    sys.exit("ERROR: Geist-var.ttf not found beside this script or in ../assets/.\n"
             "Typography is a family constant — do not substitute another face.")

# ── proportions, measured from the approved concept art ─────────────────────
# All relative to wordmark width W, so they hold at any scale.
P = dict(
    owl_w      = 0.5761,   # mark width, stacked lockup
    gap_owl    = 0.0835,   # mark bottom -> cap top
    cap_h      = 0.1702,   # wordmark cap height
    gap_desc   = 0.0491,   # baseline -> descriptor cap top
    desc_ratio = 0.20,     # descriptor size / wordmark size
    h_mark     = 1.45,     # horizontal: mark height / text-block height
    h_gap      = 0.20,     # horizontal: gap / mark width
    desc_tmin  = 0.08,     # descriptor letter-spacing floor, em
    desc_tmax  = 0.45,     # descriptor letter-spacing ceiling, em
    icon_fill  = 0.67,     # mark height / tile height, brand + mono icons
    small_fill = 0.88,     # mark height / tile height, <=32 px icon
    small_strk = 20.0,     # embolden stroke, in mark path units
    icon_rx    = 225.0,    # tile corner radius at 1024
)
W = 1000.0

# ── config ──────────────────────────────────────────────────────────────────
cfg_path = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "brand.json")
cfg = json.load(open(cfg_path, encoding="utf-8"))
OUT = os.path.dirname(cfg_path)
P.update(cfg.get("proportions", {}))
C = cfg["colors"]
WORDMARK, DESCRIPTOR = cfg["wordmark"], cfg["descriptor"]
ALT = f'{cfg["project"]} — {DESCRIPTOR.lower()}'

# ── mark geometry ───────────────────────────────────────────────────────────
mark_file = os.path.join(OUT, cfg["mark"])
if not os.path.exists(mark_file):
    sys.exit(f"ERROR: mark not found: {mark_file}\n"
             f"Draw or generate it first, then re-run. See reference/playbook.md.")

if cfg.get("mark_path_id"):
    # explicit single path, taken verbatim (preserves the author's exact data)
    raw = open(mark_file, encoding="utf-8").read()
    MARK_D = re.search(r'\sd="([^"]+)"',
                       raw[raw.index(f'id="{cfg["mark_path_id"]}"'):]).group(1)
else:
    # every shape in the file, with transforms baked in
    parts = []
    for el in SVG.parse(mark_file).elements():
        if isinstance(el, Shape):
            d = Path(el).d()
            if d:
                parts.append(d)
    if not parts:
        sys.exit(f"ERROR: no drawable shapes found in {cfg['mark']}")
    MARK_D = " ".join(parts)

mb = Path(MARK_D).bbox()
mark_w, mark_h = mb[2]-mb[0], mb[3]-mb[1]
ASPECT = mark_w / mark_h
FILL_RULE = cfg.get("mark_fill_rule", "evenodd")

# `small_strk` is a literal stroke-width in the mark file's OWN units, so what it
# means depends entirely on how big the artwork's numbers happen to be. The
# default (20) was tuned against Athena's 480-unit owl, where it is 4.17%. On a
# mark authored on a tidy 100-unit grid it becomes 25% and welds the counters
# shut — the small icon renders as a blob and nothing errors. Warn loudly rather
# than let that ship. See reference/playbook.md, "small_strk is in raw path units".
_strk_pct = P["small_strk"] / mark_w * 100
if not 400.0 <= mark_w <= 800.0:
    print(f"  WARNING: mark ink is {mark_w:.0f} units wide; the family convention is "
          f"400-800.\n"
          f"           small_strk={P['small_strk']:g} therefore embolden by "
          f"{_strk_pct:.1f}% of the mark (Athena's tuned reference: 4.17%).\n"
          f"           Re-author the mark at a larger scale, or override "
          f"proportions.small_strk,\n"
          f"           then check appicon-small.svg at true 16 px before shipping.")

# ── font ────────────────────────────────────────────────────────────────────
_f = TTFont(FONT)
UPEM, CAPH = _f["head"].unitsPerEm, _f["OS/2"].sCapHeight
face = hb.Face(hb.Blob.from_file_path(FONT))
_inst = {}
def glyphset(w):
    if w not in _inst:
        f = instantiateVariableFont(TTFont(FONT), {"wght": w}, inplace=False)
        _inst[w] = (f.getGlyphSet(), f.getGlyphOrder())
    return _inst[w]

def measure(text, wght, size):
    """Natural set width at `size`, and the glyph count."""
    font = hb.Font(face); font.set_variations({"wght": wght})
    buf = hb.Buffer(); buf.add_str(text); buf.guess_segment_properties()
    hb.shape(font, buf)
    return (sum(p.x_advance for p in buf.glyph_positions) * size / UPEM,
            len(buf.glyph_infos))

def outline(text, wght, size, target_w, x0, baseline):
    """Shape via HarfBuzz (correct kerning), stretch to exactly target_w by
    distributing the difference evenly into the letter gaps, return one path."""
    font = hb.Font(face); font.set_variations({"wght": wght})
    buf = hb.Buffer(); buf.add_str(text); buf.guess_segment_properties()
    hb.shape(font, buf)
    infos, poss = buf.glyph_infos, buf.glyph_positions
    scale   = size / UPEM
    natural = sum(p.x_advance for p in poss) * scale
    extra   = (target_w - natural) / (len(infos) - 1) if len(infos) > 1 else 0.0
    gs, order = glyphset(wght)
    out, x = [], x0
    for i, p in zip(infos, poss):
        pen = SVGPathPen(gs)
        gs[order[i.codepoint]].draw(TransformPen(pen, (
            scale, 0, 0, -scale, x + p.x_offset*scale, baseline - p.y_offset*scale)))
        if (d := pen.getCommands()):
            out.append(d)
        x += p.x_advance * scale + extra
    return " ".join(out), natural

def mark_group(fill, bx, by, bw, stroke_w=0.0):
    s  = bw / mark_w
    st = (f' stroke="{fill}" stroke-width="{stroke_w:g}"'
          ' stroke-linejoin="round" stroke-linecap="round"') if stroke_w else ""
    return (f'<g fill="{fill}" fill-rule="{FILL_RULE}"{st} '
            f'transform="translate({bx - mb[0]*s:.4f} {by - mb[1]*s:.4f}) '
            f'scale({s:.6f})"><path d="{MARK_D}"/></g>')

HEAD = ('<?xml version="1.0" encoding="UTF-8"?>\n'
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
        'width="{w}" height="{h}" role="img" aria-label="{alt}">\n  <title>{alt}</title>\n')

def write(name, body):
    open(os.path.join(OUT, name), "w", encoding="utf-8").write(body)
    print("  wrote", name)

print(f"{cfg['project']} — mark {mark_w:.1f}x{mark_h:.1f} (aspect {ASPECT:.4f})")

# ── sizing ──────────────────────────────────────────────────────────────────
# Cap height is the family constant: every project's wordmark is set at the
# same optical size, which is what makes the family read as one system. The
# lockup measure W then follows from however wide that word happens to be —
# it is NOT a fixed number. Forcing every wordmark to an identical measure
# squeezes long names and stretches short ones (OLYMPUS came out at -0.11em).
CAP_H     = P["cap_h"] * 1000.0
FONT_SIZE = CAP_H * UPEM / CAPH
W = float(cfg["wordmark_width"]) if cfg.get("wordmark_width") \
    else measure(WORDMARK, 800, FONT_SIZE)[0]

# The descriptor is flush to the wordmark, so its letter-spacing is whatever
# closing that gap requires. When the demand falls outside a legible band,
# hold the spacing at the limit and resize the descriptor instead — spacing
# is the thing a reader notices, size is not.
d_unit, d_glyphs = measure(DESCRIPTOR, 600, 1.0)
d_gaps    = max(d_glyphs - 1, 1)
DESC_SIZE = FONT_SIZE * P["desc_ratio"]
track     = (W - d_unit*DESC_SIZE) / d_gaps / DESC_SIZE
if track > P["desc_tmax"]:
    DESC_SIZE, track = W / (d_unit + d_gaps*P["desc_tmax"]), P["desc_tmax"]
    print(f"  descriptor too short for the measure — resized, spacing held at {track:.3f}em")
elif track < P["desc_tmin"]:
    DESC_SIZE, track = W / (d_unit + d_gaps*P["desc_tmin"]), P["desc_tmin"]
    print(f"  descriptor too long for the measure — resized, spacing held at {track:.3f}em")
print(f"  measure W={W:.1f}  cap={CAP_H:.1f}  descriptor {DESC_SIZE:.2f} @ {track:.3f}em")

# ── stacked lockup ──────────────────────────────────────────────────────────
# Mark size and gaps key off cap height (via the fixed reference measure REF),
# never off W. Keying them to W would grow Olympus's mark simply because
# OLYMPUS is a longer word than ATHENA — the marks must stay the same size
# across the family. A longer name just makes a wider lockup.
REF      = 1000.0
MW       = P["owl_w"] * REF
mh       = MW / ASPECT
DESC_CAP = DESC_SIZE * CAPH / UPEM
base  = mh + P["gap_owl"]*REF + CAP_H
dbase = base + P["gap_desc"]*REF + DESC_CAP
word_d, _ = outline(WORDMARK,   800, FONT_SIZE, W, 0.0, base)
desc_d, _ = outline(DESCRIPTOR, 600, DESC_SIZE, W, 0.0, dbase)

# Everything below is placed by measured INK, never by advance width. A glyph's
# advance includes its side bearings, so a wordmark set from x=0 to x=W does not
# have its visible ink centred on W/2 — centring the mark on the canvas instead
# of on the ink leaves it visibly off. Same reasoning drives the viewBox: it is
# the union of the ink, so the file has no dead margin to misalign against.
def ink(*ds):
    bs = [Path(d).bbox() for d in ds if d]
    return (min(b[0] for b in bs), min(b[1] for b in bs),
            max(b[2] for b in bs), max(b[3] for b in bs))

def emit(name, alt, mark_fill, mark_box, word_fill, word_d, desc_fill, desc_d):
    """mark_box = (x, y, width) of the mark's ink."""
    mx, my, mwid = mark_box
    mb_ = (mx, my, mx + mwid, my + mwid/ASPECT)
    tb  = ink(word_d, desc_d)
    x0, y0 = min(mb_[0], tb[0]), min(mb_[1], tb[1])
    x1, y1 = max(mb_[2], tb[2]), max(mb_[3], tb[3])
    write(name, ('<?xml version="1.0" encoding="UTF-8"?>\n'
                 f'<svg xmlns="http://www.w3.org/2000/svg" '
                 f'viewBox="{x0:.2f} {y0:.2f} {x1-x0:.2f} {y1-y0:.2f}" '
                 f'width="{x1-x0:.2f}" height="{y1-y0:.2f}" '
                 f'role="img" aria-label="{alt}">\n  <title>{alt}</title>\n'
                 f'  {mark_group(mark_fill, mx, my, mwid)}\n'
                 f'  <path fill="{word_fill}" d="{word_d}"/>\n'
                 f'  <path fill="{desc_fill}" d="{desc_d}"/>\n</svg>\n'))

MARK_LIGHT = C.get("accent_on_light", C["accent"])
tcx = (lambda b: (b[0]+b[2])/2)(ink(word_d, desc_d))   # text ink centre

print("stacked lockup")
for name, mk, wc, dc in (("logo-light.svg", MARK_LIGHT,  C["ink"],   C["accent_deep"]),
                         ("logo-dark.svg",  C["accent"], C["paper"], C["accent"])):
    emit(name, ALT, mk, (tcx - MW/2, 0.0, MW), wc, word_d, dc, desc_d)

# ── horizontal lockup ───────────────────────────────────────────────────────
text_h = CAP_H + P["gap_desc"]*REF + DESC_CAP
hmh    = text_h * P["h_mark"]
hmw    = hmh * ASPECT
hbase  = (hmh - text_h)/2 + CAP_H
hdbase = hbase + P["gap_desc"]*REF + DESC_CAP
_w0, _ = outline(WORDMARK,   800, FONT_SIZE, W, 0.0, hbase)
_d0, _ = outline(DESCRIPTOR, 600, DESC_SIZE, W, 0.0, hdbase)
# shift the text so the gap is measured ink-edge to ink-edge, then re-shape
shift = hmw + hmw*P["h_gap"] - ink(_w0, _d0)[0]
hword_d, _ = outline(WORDMARK,   800, FONT_SIZE, W, shift, hbase)
hdesc_d, _ = outline(DESCRIPTOR, 600, DESC_SIZE, W, shift, hdbase)
# and vertically centre the text ink against the mark ink
htb  = ink(hword_d, hdesc_d)
dy   = (hmh - (htb[3]-htb[1]))/2 - htb[1]
hword_d, _ = outline(WORDMARK,   800, FONT_SIZE, W, shift, hbase + dy)
hdesc_d, _ = outline(DESCRIPTOR, 600, DESC_SIZE, W, shift, hdbase + dy)

print(f"horizontal lockup")
for name, mk, wc, dc in (("logo-h-light.svg", MARK_LIGHT,  C["ink"],   C["accent_deep"]),
                         ("logo-h-dark.svg",  C["accent"], C["paper"], C["accent"])):
    emit(name, ALT, mk, (0.0, 0.0, hmw), wc, hword_d, dc, hdesc_d)

# ── app icons ───────────────────────────────────────────────────────────────
S = 1024.0
def icon(g0, g1, edge, mark, fill, rx, stroke_w=0.0, full_bleed=False):
    mw_ = (fill*S) * ASPECT
    g   = mark_group(mark, (S-mw_)/2, (S - fill*S)/2, mw_, stroke_w)
    if g1:
        defs = ('  <defs><linearGradient id="t" x1="0" y1="0" x2="0" y2="1">'
                f'<stop offset="0" stop-color="{g0}"/><stop offset="1" stop-color="{g1}"/>'
                '</linearGradient></defs>\n')
        tile = "url(#t)"
    else:
        defs, tile = "", g0
    if full_bleed:
        # genuinely edge-to-edge: no inset, no edge stroke. The OS applies its
        # own mask at small sizes and every pixel counts.
        rect = (f'  <rect x="0" y="0" width="1024" height="1024" '
                f'rx="{rx:g}" ry="{rx:g}" fill="{tile}"/>')
    else:
        # inset by half the stroke so the edge stroke reads as a defined border
        rect = (f'  <rect x="8" y="8" width="1008" height="1008" '
                f'rx="{rx:g}" ry="{rx:g}" fill="{tile}" stroke="{edge}" stroke-width="16"/>')
    return (HEAD.format(w="1024", h="1024", alt=cfg["project"]) + defs +
            rect + f'\n  {g}\n</svg>\n')

IF, RX = P["icon_fill"], P["icon_rx"]
tl, td = C["tile_light_chrome"], C["tile_dark_chrome"]
print(f"app icons  mark fills {IF:.0%}")
write("appicon-light.svg", icon(tl[0], tl[1], tl[2], C["accent"],        IF, RX))
write("appicon-dark.svg",  icon(td[0], td[1], td[2], C["accent_bright"], IF, RX))
write("appicon-mono-light.svg", icon("#ffffff", None, "#dfe3e8", "#000000", IF, RX))
write("appicon-mono-dark.svg",  icon("#000000", None, "#2a2f36", "#ffffff", IF, RX))

# A mark with fine interior detail can be illegible at 16 px no matter how it is
# coloured or emboldened — the problem is geometry, not contrast. `mark_small`
# lets a project supply a simplified mark used ONLY here.
if cfg.get("mark_small"):
    _p = os.path.join(OUT, cfg["mark_small"])
    if not os.path.exists(_p):
        sys.exit(f"ERROR: mark_small not found: {_p}")
    MARK_D = " ".join(d for e in SVG.parse(_p).elements()
                      if isinstance(e, Shape) and (d := Path(e).d()))
    mb = Path(MARK_D).bbox()
    mark_w, mark_h = mb[2]-mb[0], mb[3]-mb[1]
    ASPECT = mark_w / mark_h
    print(f"small variant  using simplified mark {cfg['mark_small']} "
          f"({mark_w:.0f}x{mark_h:.0f}, aspect {ASPECT:.4f})")
else:
    print(f"small variant  mark fills {P['small_fill']:.0%}, stroke {P['small_strk']:g}")
write("appicon-small.svg", icon(C["tile_small"], None, C["tile_small"], "#ffffff",
                                P["small_fill"], 0.0, P["small_strk"], full_bleed=True))
