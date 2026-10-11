# A background pack that draws on the GPU

* **Status:** **RULED 2026-09-20 — `.rulings/` R-206. §5 is B**, a pack
  declares GPU-drawing in its `Description`, a pack may also carry a CPU path,
  and `devices` gains a token for a render node without a screen. Being built.
* **Date:** 2026-09-20
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Why this exists now

Two of `T8`'s four new backgrounds do not fit the frame budget, **measured on
`rivet-01`**:

```
cost over 9000 frames drawing Contours:
  background 32.12/38.18 ms, widgets 9.65/11.59 ms
  DRAWING 41.77 ms of the 33.3 ms budget (125%), frame 50.09 ms -> 20.0 fps
```

2.6x Digital Rain's 12.15 ms, and the panel has fallen to every third vblank.
`PLAN-Addon-Feature-Set` T8's own standard is that **a background that misses
the budget is a defect, not a preference**, so Contours does not ship as it
stands and Aurora — unmeasured, 3.9x Digital Rain on x86 — is expected to join
it.

**The two that fail are the two a GPU would make nearly free.** Aurora and
Contours are both a 96x54 scalar field sampled per pixel and scaled to two
megapixels. On the V3D that is one textured quad with the texture unit doing
the bilinear filtering in hardware — the single most GPU-shaped workload in
this repository — and the ARM cores are spending 32 ms a frame on it.

The maintainer's framing, 2026-09-20, and it is the stronger argument: these
work as even better examples if the project is really trying to use the hardware
(`.rulings/` R-206). A public Rivet that ships `mesa-demo` proving `GL_RENDERER = V3D 4.2` and then
an example kiosk application that writes every pixel with the CPU is a
platform arguing against itself.

---

## 1.1 THREE backgrounds miss the budget, not two — measured 2026-09-20

§1 above named Aurora and Contours. **Conway's Game of Life is a third**, and
nobody knew because it had never been measured. Asked by the maintainer — how
about Conway's Game of Life, then (`.rulings/` R-320) — and answered by putting it on
`rivet-01` and reading its own report:

```
cost over 9000 frames drawing `life/pulse`:
  background 26.15/49.56 ms, widgets 8.91/13.73 ms
  DRAWING 35.07 ms of the 33.3 ms budget (105%)
```

The whole CPU picture, on one board, one driver, one compositor:

| background | CPU background | verdict |
| :-- | :-- | :-- |
| Contours / Ice Blue | **34.29 ms** | over |
| Life / Pulse | **26.15 ms** | **over — newly known** |
| Digital Rain | 12.15 ms | fits |
| Aurora | never measured here | expected over; **6.76 ms on the GPU** |

**Life is over budget on its DRAWING and not on its simulation**, which is the
part that decides how to fix it. The board is 160x90 and steps four times a
second — 57,600 neighbour sums a second, nothing. What costs is the frame:
`LIFE_FADE_SECONDS` is 0.9, so cells fade continuously and the picture changes
on every frame even between steps. Each frame clears two megapixels and then
draws a shape per live cell. `Pulse` is the most expensive of the three styles
because its radius varies with age, and it is also the one the maintainer
judged worth having above the others (`.rulings/` R-320).

**And it ports by exactly Aurora's mechanism.** Upload the 160x90 heat grid as
a texture; the fragment shader derives which cell a fragment is in, its
position within that cell, and the shape from the cell's heat. One textured
quad, no instancing, no per-cell draw call — the same shape as Aurora's field
plus a modulo. Of the four field-ish backgrounds this is the *least* work to
move, not the most.

---

## 1.2 AND THEN THE PREMISE MOVED: it was one slow primitive, not seven expensive backgrounds

**Measured on `rivet-01`, 2026-09-20, after `rivet_sdk::draw::clear` and
`disc_coverage`'s `sqrt`:**

| background | before | after | budget |
| :-- | --: | --: | --: |
| `contours/ice-blue` | 34.29 ms | **13.34 ms** | 132% → **75%** |
| `life/pulse` | 26.15 ms | **11.69 ms** | 105% → **66%** |
| `starfield/white` | 21.53 ms | **4.11 ms** | 95% → **48%** |

**All three fit, on the CPU, with no GPU involved.**

**Starfield is what gave it away**, and the maintainer's reaction is the reason
anybody looked: he found it a weak starfield, with barely any stars visible, and
boring (`.rulings/` R-207). It draws 320 stars of ONE PIXEL each and cost 21.53 ms. Three hundred
and twenty single pixels cannot cost that — so the stars were free and the
clear was the whole bill, and the same clear sat in front of every other
background. §1's three numbers were never measuring the backgrounds.

Two primitives, both in `rivet-sdk/src/draw.rs`, both shared by everything:

* **`fill` over the whole surface.** It is `blend` at full opacity — an
  identity its own test asserts byte for byte — so it preserves every fourth
  byte, which is exactly what forbids a memset. `draw::clear` is a separate
  operation that writes all four (XRGB padding the scanout ignores). Worth
  about **17 ms a frame**, measured cleanly on Starfield, which is
  deterministic.
* **`disc_coverage`'s `f32::hypot`**, an out-of-line libm call on
  musl/aarch64, run once per pixel of every disc's bounding box. Now `sqrt`,
  and the byte-identity guard still passes.

**What is NOT separated, said plainly:** Life carries both changes and its
cost is population-dependent — the soup burns down, the watchdog injects — so
`26.15 → 11.69` is not a clean attribution between the two, and the `sqrt`
change has no isolated measurement. Starfield's and Contours' numbers are
clean; Life's is directional.

**This does not retire the GPU work.** Contours is 8.19 ms there against 13.34
here, and the point was never only to fit — it is headroom. What changes is the
framing: these were not seven expensive backgrounds, and `STAR_COUNT = 320` at
one pixel is what fits when twenty milliseconds are gone before you start.

---

## 1.3 EVERY background measured, both ways — 2026-09-20

All on `rivet-01`, 1920x1080, V3D 4.2.14.0. **Background phase only**, which is
the comparable quantity: the CPU column is what the pack's `draw` call costs
inside `aura-renderer`, the GPU column what the same picture costs in
`examples/probes/gpu-surface`.

| background | CPU before | CPU after | GPU | GPU vs CPU-after |
| :-- | --: | --: | --: | --: |
| `contours/ice-blue` | 34.29 ms | 13.34 ms | **8.19 ms** | **1.6x** |
| `life/pulse` | 26.15 ms | 11.69 ms | **9.80 ms** | **1.2x** |
| `boxes/ice-blue` | — | 11.38 ms | **8.93 ms** | **1.3x** |
| `digital-rain/phosphor-green` | 12.15 ms | 8.52 ms | — | — |
| `aurora/green-violet` | — | — | **6.76 ms** | *no CPU number* |
| `starfield` | 21.53 ms | 4.11 ms | **3.86 ms** | **1.06x — a tie** |

"CPU before" is a full-surface `draw::fill` plus `hypot` in `disc_coverage`;
"CPU after" is `draw::clear` plus `sqrt` (§1.2).

### What the GPU column actually decomposes into

**Starfield's 3.86 ms is the FLOOR**, not a result. Its `GL_POINTS` draw covers
about four hundred fragments, so what it measures is the fixed cost of putting
any frame on this surface at all — clear, the 8 MB tile store at 1080p,
`glFinish`, and the buffer hand-over. Every other GPU number is that floor plus
the shader:

```
starfield  3.86   the floor
aurora     6.91   + two texture fetches per fragment
contours   9.56   + a fetch, fwidth and a compare per fragment
boxes      8.93   + a fetch and the pixel-space division per fragment
life       9.86   + a fetch and a length() per fragment
```

**So the V3D charges about 4 ms to write two megapixels before any shader runs,
and 3-6 ms for the shader.** That is the number that decides where R-206
applies, and it could not have been known without Starfield.

### The conclusion, which is narrower than §1 assumed

**The GPU is worth it where there is PER-PIXEL WORK to move, and nowhere
else.** Contours earns 1.6x because it computes a gradient and a distance for
every one of two million pixels. Starfield is a **tie** — it has no per-pixel
work at all, and porting it buys a fixed overhead in exchange for nothing.

**And the CPU fixes changed the answer.** Against the *old* CPU numbers the GPU
looked like a 4.2x rescue; against the fixed ones it is 1.2x to 1.6x. Every
background now fits on the CPU, so **the GPU is headroom, not rescue** —
valuable for making a background richer than the CPU could afford, which is the
argument Starfield's `STAR_COUNT = 320` at one pixel makes better than any
measurement here.

---

## 1.4 What ships, and what Starfield spent its headroom on — `.rulings/` R-207

**GPU:** `contours`, `life`, `boxes`. **CPU:** `digital-rain`, `starfield`.
**Skipped:** `aurora` — the fastest thing measured here and the one with no CPU
number to be a speedup over.

**Starfield stays on the CPU deliberately**, so the pack keeps a worked example
of each path, and it ties on the GPU anyway (3.86 against 4.11). Its headroom
went into the picture instead:

```
320 stars, 1-2 px squares      background  4.11 ms   12% of budget
2,400 stars, discs to ~5 px    background  8.72 ms   26% of budget
```

7.5x the stars, a size that carries depth, and brightness as the disc's alpha
rather than a darker colour — inside a 10 ms ceiling. **That trade is the
reason the CPU fixes mattered more than the GPU port did**: what §1 read as
seven backgrounds needing rescue was one slow primitive, and what it bought
back was not speed but the freedom to draw something better.

---

## 2. What is already true, and it is more than expected

Nothing below is proposed. It is all in the tree today.

| piece | state |
| :-- | :-- |
| V3D in the device tree | built in; `tools/card/build.sh` **refuses a card** whose V3D block is disabled |
| Mesa in the image | `BR2_PACKAGE_MESA3D=y`, `BR2_PACKAGE_MESA3D_GALLIUM_DRIVER_V3D=y` |
| `libEGL` / `libGLESv2` / `libgbm` on the card | collected by `collect_libraries`' NEEDED closure |
| GBM's `dlopen`ed backend | staged by `collect_gbm_backends`, after a card once shipped with `gbm_create_device` returning NULL |
| shaders run on the hardware | `mesa-demo`: `GL_RENDERER = V3D 4.2` |
| an application may load them | `PUBLIC_LIBRARIES` carries all three — **`library = []` stays true** |
| the backend is reachable | `confinement::GBM_BACKEND_DIR`, 2026-09-20, after `gpu-flip` was refused with `Permission denied` |
| KMS page-flip | `rivet_sdk::screen`, months on the wall |

**The one thing not yet established** is that a `gbm_bo` can become a DRM
framebuffer a leased CRTC will flip to. `examples/probes/gpu-flip` asks exactly
that and nothing else.

### 2.1 An application cannot open a card, and does not need to

`rivet_core::service::device_globs` expands a declared `display` to
`/dev/dri/renderD*` for a `DisplayRole::Lessee` and to the card only for the
`Arbiter` — which is *"not written in a manifest and must not be"*, one per
machine, today `rivet-launcher`. **No application will ever open
`/dev/dri/card0`**, and `ADR-015` §6.2 is why: one that could ask to be the
arbiter could ask for the screen.

A lessee flips anyway, through the **lease descriptor** handed over the socket
`RIVET_DISPLAY` names. That descriptor is a DRM fd: GBM makes a device on it
and KMS flips the objects it carries. The first draft of `gpu-flip` opened a
card and was wrong; the corrected one is an ordinary kiosk application that
happens to draw with the GPU, which is the better thing to prove.

---

## 3. The trap that decides the shape

**A half-GPU renderer is slower than an all-CPU one.**

If a background draws on the V3D and the widgets stay on the CPU, the finished
frame has to come *back* from GPU memory to be composited. A readback of
8.29 MB at 1080p costs more than the 32 ms being saved. The win exists only if
the frame **stays** on the GPU and is scanned out of the buffer the GPU drew
into.

So "move Contours to the GPU" is not a change to one background. It is:

1. `rivet_sdk::screen` gains a GBM/EGL path beside its dumb buffers;
2. `aura-renderer` composites its widgets with GL rather than with
   `rivet_sdk::draw`;
3. the addon boundary grows a way for a pack to draw into a GPU surface.

(1) is assembly — `gpu-flip` is the proof. (2) is ordinary work of real size:
`widgets.rs` and `layout.rs` are 3,400 lines of CPU rasterisation and text.
(3) is the part that needs a decision, and it is §5.

---

## 4. What a cadence field would and would not buy

Asked 2026-09-20: *does `aura-renderer` have a mechanism for requiring a
specific cadence from an addon to maintain fps?* **It does not.**
`rivetos_addon::frame::Frame` carries `abi, width, height, stride, format,
reserved, pixels, time_ms, background, theme` — no budget, no deadline, no
cadence — and `Loaded::draw` is a synchronous call with no timeout. A pack that
takes 500 ms a frame makes the wall run at 2 fps and nothing notices.

`Theme::animated` is the only adjacent field and it is binary: redraw every
frame, or do not.

**A cadence field would not make an over-budget background acceptable**, and
that is the part to be clear about before it becomes a loophole. If Aurora
takes 45 ms, the frame it draws on overruns 33.3 ms and misses its flip whether
that happens 30 times a second or 15. Cadence reduces *average* load and power;
the budget is *per drawn frame*. Life wants it (it steps at 4 Hz by design) and
`Background-Addons-2026-09-19` §6 raised it as an open question. It is worth
building. It is not a way to ship something that does not fit.

---

## 5. THE MAINTAINER'S: how does a GPU pack cross the boundary?

`rivet-addon/src/frame.rs` spends its longest passage arguing that the
`#[repr(C)]` descriptor is a **deliberate, narrow exception** to the protocol's
first rule that no type layout crosses — narrow because it is fixed-width
scalars and a raw pointer, which *is* the platform's C ABI. A GL context is not
narrow. `Frame { pixels: *mut u8 }` is a CPU surface by construction.

Three shapes, with what each costs.

### A. A second entry point, `rivet_addon_draw_gl`

The descriptor grows a sibling carrying an `EGLDisplay`, an `EGLContext` and a
framebuffer object name, and a pack exports either entry point or both.
`Description` says which.

* **For:** CPU packs are untouched and keep working. A pack chooses.
* **Against:** it puts an EGL handle across a `dlopen` boundary, which is a
  much larger contract than four integers and a pointer — two separately
  compiled objects now have to agree about a driver's state machine. It also
  makes the host's GL state the pack's problem: a pack that leaves a bound
  texture or a changed blend mode corrupts the widgets, and nothing at the
  boundary can prevent it.

### B. The pack renders to a texture the host composites

The host creates a texture, the pack is handed its name plus a framebuffer
already bound, and draws into it. The host owns the context, the state, and the
compositing.

* **For:** the state machine stays the host's and the contract is "here is a
  bound target, draw"; it is the same shape as today's surface, one level up.
* **Against:** still an EGL/GL dependency in every pack that wants it, and the
  pack must not assume anything about the context it did not create.

### C. Packs stay on the CPU; the HOST moves to the GPU

The renderer composites with GL, uploads each pack's CPU frame as a texture,
and draws it as a quad.

* **For:** the addon ABI does not change at all. Every existing pack keeps
  working with no edit.
* **Against:** **it does not fix Contours.** The pack still spends 32 ms
  writing two megapixels on the ARM cores, and the upload is added on top. It
  buys a GPU widget compositor and nothing for the backgrounds that fail.

### The recommendation

**B**, and it is not close. It keeps the property `frame.rs` bought the
exception for — the host owns the surface and the pack writes into what it is
handed — while changing what "the surface" is. A is the same contract with the
driver's state machine added to it, for no gain a pack can use; C is honest
about compatibility and does not solve the problem that started this.

**What B still leaves open, and should be ruled with it:** whether a pack
declares GPU-drawing in its `Description` (so a host can refuse gracefully and
a CPU-only host can skip it), and whether a pack that draws on the GPU may also
offer a CPU path for hosts that have no V3D. The second matters for the x86
target named in `rivet-long-term-targets` — a machine with no V3D would
otherwise have no backgrounds at all.

---

## 6. Order

1. **`gpu-flip` on hardware.** One unknown, two seconds of runtime. If a
   `gbm_bo` will not flip, everything above is moot and that is worth knowing
   before any of it is built.
2. **§5 ruled.**
3. `rivet_sdk::screen`'s GBM path.
4. `aura-renderer` composites with GL.
5. Aurora and Contours re-measured against the 33.3 ms budget, on the board.
6. A 24-hour thermal soak on a GPU background — the maintainer asked for one,
   and the CPU baseline it compares against is running on `rivet-01` now
   (Contours at 125%, which is the worst case the CPU can produce).
