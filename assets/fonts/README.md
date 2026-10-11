# The system typefaces

`Geist.ttf` is **Geist**, by Vercel, under the SIL Open Font License 1.1 —
`OFL.txt` beside it is that licence and travels with the file wherever it goes.

`GeistMono.ttf` is **Geist Mono**, and **`OFL.txt` covers it too**: upstream
ships both faces from one project under one copyright line, and the file here is
byte-identical to `vercel/geist-font`'s own `OFL.txt` — checked with `cmp`, not
assumed. One licence file, two faces, no second copy to fall out of date.

## Why there is a monospace at all

Added 2026-09-04, ruled by the maintainer. A monospace is not a style
preference here. Every Rivet screen that shows a **number changing in place** —
a clock, a temperature, a byte count, a percentage — wants digits that hold
their column as the value changes, and a proportional face cannot give that.
Geist's digits are proportional, so a clock drawn in it shifts sideways under
itself once a second: legible, and it looks broken.

**Geist Mono rather than Source Code Pro**, which Aura already ships and which
is equally free to redistribute. Two reasons. It is the same family as the sans
already in the image, and pairing Geist with an unrelated mono puts two
typefaces on one screen for no reason. And it is the mono the maintainer's other
projects use, which is the same argument that chose Geist in the first place.

Aura keeps Source Code Pro — its sizes were chosen against those metrics over
months on a wall, and `examples/aura/aura-renderer/assets/README.md` makes that
case. A system mono existing obliges nobody to draw with it.

## Why this one

`PLAN.md` §104.38. It is the typeface the maintainer's other projects already
use — Athena, Olympus, Ignition, Tachyon and Trends — so a Rivet screen matches
everything else he builds rather than being the one thing that looks different.

## Why they are committed rather than fetched

`tools/card/build.sh` stages them into the image at `rivet_core`'s font paths,
and a build that reached outside the repository for them would be one nobody
else could reproduce — the argument `tools/card` already makes about every other
input. 169 KB and 172 KB are a cheap price for a build that does not depend on a
path in one person's home directory.

**Both are variable fonts**, and `fontdue` rasterises the default instance.
Naming a weight is a thing to do the day a screen needs two of them.

### Where these exact bytes came from

Both are upstream's own variable builds, so the provenance can be re-checked
rather than taken on trust:

| file here | upstream path in `vercel/geist-font` | bytes |
| :-- | :-- | --: |
| `Geist.ttf` | `fonts/Geist/variable/Geist[wght].ttf` | 169,056 |
| `GeistMono.ttf` | `fonts/GeistMono/variable/GeistMono[wght].ttf` | 171,968 |

`Geist.ttf` had been committed before that path was written down; the size
matches upstream's byte for byte, which is what identified it. `GeistMono.ttf`
was fetched from that path on 2026-09-04.

**`docs/brand/Geist-var.ttf` is the same 169,056 bytes** — verified identical
with `cmp`. Two copies of one file is not ideal, but they serve different
consumers: one is staged into a running image and the other is a brand asset
beside the logo work. Neither is generated from the other, so a divergence
would be silent — worth a check the day either is replaced.
