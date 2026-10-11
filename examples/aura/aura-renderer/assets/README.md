# Aura's typefaces, shipped in the package

Three faces, carried in the `.rvt` as `ADR-015` §3's *"optional assets"* and
unpacked flat beside the program, where `RIVET_APP_DIR` points — plus the
licences that must travel with them.

| file | what it draws | atlas size in the C++ |
| :-- | :-- | --: |
| `SourceCodePro-Bold.ttf` | the clock, and every number on every card | 292 |
| `SourceCodePro-Regular.ttf` | the date, the forecast statement, the units | 60 |
| `MaterialSymbols-Aura.ttf` | the card icons — see `src/icons.rs` | — |

| licence file | covers | terms |
| :-- | :-- | :-- |
| `SourceCodePro-LICENSE.txt` | both Source Code Pro weights | SIL OFL 1.1 |
| `MaterialSymbols-LICENSE.txt` | the icon subset | Apache 2.0 |

## `AuraRain.ttf` left this package on 2026-09-20

**It is now `examples/aura/aura-background-pack/assets/`, with
`IPAFont-LICENSE.txt` beside it**, because Digital Rain is the pack's and this
program no longer rasterises a single one of those glyphs. The pack embeds the
bytes in its own object rather than reading them from a path — a shared library
cannot assume a `RIVET_APP_DIR`, since the directory belongs to whichever host
loaded it.

**The licence moved with the font, which is the point.** IPA Font License 1.0
§3.1(3) requires the licence text to accompany the derived program; leaving a
copy here would have meant an IPA-licensed file in an artifact that uses
nothing from it, and leaving the licence here without the font would have been
worse. The whole account below — the subsetting recipe, the rename and the
proof the outlines are unchanged — went with them and is now that package's
`assets/README.md`.

The sections that follow are kept only as this package's own history of a file
it used to carry.

**All three licence files were missing until 2026-09-04**, and the fonts had
been shipping without them since Aura was written. This README named the
licences; naming a licence is not carrying one, and two of these three require
the text to accompany the font. The same gap exists upstream in `Aura/assets/`,
which is where these files were copied from — fixing it there is a separate job
in a separate repository.

## Why they are in the package and not in the image

`rivet_sdk::text::SYSTEM_FONT` is **Geist**, and it is the right typeface for a
Rivet screen — the launcher, the splash and every template draw with it, and it
belongs to the image because several programs share it.

It is the wrong typeface for Aura. These are the faces Aura's numbers have been
set in on a wall for months, and every size in `cpp/src/widgets.hpp` was chosen
against their metrics. `ADR-015` §3 makes a package *"one static binary, a typed
manifest, optional assets, and a signature"* precisely so an application can
carry what is its own rather than asking the platform to carry it.

## Provenance and licences

**Source Code Pro** is Adobe's, under the SIL Open Font License 1.1. Both files
are copied byte for byte from `Aura/assets/`, which is where they have been
since Aura was written.

**`AuraRain.ttf` is a SUBSET** of `Aura/assets/IPAGothic.ttf` — IPA Gothic, by
the Information-technology Promotion Agency, Japan, under the IPA Font License
1.0. It is cut to the 105 codepoints `cpp/src/ss_matrix.hpp` puts in its
character pool: the ten digits, `A`–`Z`, `a`–`z`, and the 43 katakana that file
lists by codepoint.

### Why it is not called `IPAGothic-Aura.ttf` any more

**It was, until 2026-09-04, and that name broke the licence.** IPA Font License
1.0 Article 3.1(4), on a Derived Program — which a subset is — reads:

> No one may use or include the name of the Licensed Program as a program name,
> font name or file name of the Derived Program.

The old file was named `IPAGothic-Aura.ttf` and its internal `name` table still
read `IPAGothic` in every one of IDs 1, 3, 4 and 6. That is the prohibited name
in two of the three places the Article lists, and it was found while adding the
licence file that Article 3.1(3) requires — the licence text and the breach
arrived in the same reading.

So the file is `AuraRain.ttf`, its family is `Aura Rain` and its PostScript name
is `AuraRain-Regular`. **The outlines did not change**: re-checked with
`fontTools` against the file it replaces — same 105 codepoints, `unitsPerEm`
2048, `hhea` 1802/−246, and zero mismatches across every `hmtx` advance and
every `glyf` coordinate. Digital Rain draws exactly what it drew before.

The other obligations, so the next reader can check them rather than trust this:

* **3.1(1)** — the derived program ships with the package, and no intermediate
  file was produced in making it.
* **3.1(2)** — a recipient must be able to get back to the original. The
  subsetting command below is that method: it names the upstream face and
  reproduces this file from it, so the original is identified and the difference
  is a command rather than a mystery.
* **3.1(3)** — `IPAFont-LICENSE.txt` is beside the font and this derived work is
  under those same terms.

**A cheaper way out exists and was not taken.** Replacing IPA Gothic with an
OFL-licensed Japanese face — Noto Sans JP — would remove this Article from the
picture entirely. It would also change what Digital Rain looks like, and that is
a wall the maintainer has watched for months. Keeping the shapes and fixing the
name is the smaller change.

The full face is 6.2 MB for 12,728 glyphs and Digital Rain draws 105 of them.
Subsetting is the same move Aura already makes for its icon face —
`MaterialSymbols-Aura.ttf` is a subset for the same reason — and it takes the
file to 19 KB.

**The subset is exact, and that was checked rather than assumed.** With
`fontTools`, against the original: `unitsPerEm` unchanged at 2048, `hhea`
ascent/descent/lineGap unchanged at 1802/−246/0, and for all 105 codepoints the
`hmtx` advance and the `glyf` outline coordinates identical. Zero mismatches.
Those are the numbers the rasteriser scales by, so a glyph drawn from this file
is the glyph that would have been drawn from the whole face.

The command that produced it, so it can be produced again:

```
uv run --with fonttools python -c '
from fontTools import subset
kata = [0x30A1,0x30A2,0x30A3,0x30A4,0x30A5,0x30A6,0x30A7,0x30A8,0x30A9,0x30AA,
        0x30AB,0x30AC,0x30AD,0x30AE,0x30AF,0x30B1,0x30B3,0x30B5,0x30B7,0x30B9,
        0x30BB,0x30BD,0x30BF,0x30C1,0x30C4,0x30C6,0x30C8,0x30CA,0x30CB,0x30CC,
        0x30CF,0x30D2,0x30D5,0x30D8,0x30DB,0x30DE,0x30DF,0x30E0,0x30E4,0x30E6,
        0x30E8,0x30EF,0x30F3]
pool = [ord(c) for c in "0123456789"] + list(range(65,91)) + list(range(97,123)) + kata
subset.main(["IPAGothic.ttf", "--output-file=AuraRain.ttf",
             "--unicodes=" + ",".join(f"U+{c:04X}" for c in pool),
             "--no-recalc-bounds", "--notdef-outline",
             "--drop-tables+=DSIG", "--glyph-names"])'
```

**Then the rename, which Article 3.1(4) requires** — a subset alone still calls
itself `IPAGothic` inside, so this second step is not cosmetic:

```
uv run --with fonttools python -c '
from fontTools.ttLib import TTFont
f = TTFont("AuraRain.ttf")
new = {1: "Aura Rain", 2: "Regular", 3: "Aura Rain; a 105-glyph derived program",
       4: "Aura Rain", 6: "AuraRain-Regular"}
for r in f["name"].names:
    if r.nameID in new: r.string = new[r.nameID]
f["name"].names = [r for r in f["name"].names if r.nameID not in (16,17,18,20,21,22)]
f.save("AuraRain.ttf")'
```

Together these two commands are also the Article 3.1(2) answer: they name the
Original Program and state the method, so a recipient holding `AuraRain.ttf` can
identify what it came from and go back to it.

## What is NOT here, and why

**The full `IPAGothic.ttf`** — nothing needs the other 12,623 glyphs. If a
future card carries Japanese text the subset grows, and the command above is
how.
