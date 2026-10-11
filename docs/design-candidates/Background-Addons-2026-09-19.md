# Background addons: the default multipack, and four new ones

* **Status:** Design candidate, **ruled and partly built**. `.rulings/` R-199
  decided it; `docs/PLAN-Addon-Feature-Set-2026-09-19.md` is the execution
  queue and the record of what each task actually built. **Read the plan
  before treating any sketch below as a specification** — §3.1's enumeration
  channel (T2) and §4's frame call (T3) are built, and T3 departed from §4's
  sketch in ways its entry explains. `rivet-addon/src/frame.rs` is the
  specification for the crossing.
* **Date:** 2026-09-19
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Rests on:** the addon protocol (`.rulings/` R-196 to R-198,
  `docs/design-candidates/Addon-Call-Boundary-2026-09-19.md`), proven on
  `rivet-06f5c` the same day.

---

## 1. Where the screensavers actually live

The maintainer asked for them to be extracted from `aura-controller`. **They
are not in it.** `SPEC-04` §12 records where they are: Digital Rain, Boxes and
Solid were ported constant for constant from the C++ `ss_matrix.hpp`,
`ss_boxes.hpp` and `ss_solid.hpp` into **`aura-renderer`**, which is the half
that owns a display.

What the controller holds is the **selection**, and not even as two fields: its
Go original folds *Digital Rain* plus *Ice Blue* into a single value like
`construct_ice`, and the Rust port kept that. Grepping for a theme on the wire
finds nothing, which has misled a reader before.

So the work splits across two hosts, and the split is the interesting part of
this design rather than a complication.

## 2. Two hosts, one object

| | loads it to | asks it |
| :-- | :-- | :-- |
| **`aura-renderer`** | **draw** | the frame call (§4) |
| **`aura-controller`** | **describe** | `Describe`, once, for the portal's two pickers |

Both declare `accepts-class = ["aura-background"]`, so `ADR-053`'s class match
puts the same object on both `LD_LIBRARY_PATH`s and each opens it for its own
purpose. `Describe` is cheap and has no side effect, so a controller that never
draws a pixel can still tell the operator what backgrounds exist and what
themes each one has — which is exactly the picker the maintainer described.

**Nothing new is needed for this.** It is two applications accepting one class,
which the platform already does.

## 3. A multipack is the natural unit, and `ADR-053` already allows it

`provides-class` is one string per library because **a library provides one
kind of thing** — one *kind*, not one *instance*. A pack of class
`aura-background` describing four backgrounds, each with its own themes, is one
library providing one kind of thing. `ADR-053` is untouched.

The description grows a list:

```rust
pub struct Backgrounds {
    pub backgrounds: Vec<Background>,
}
pub struct Background {
    pub id: String,          // stable, machine-read: "digital-rain"
    pub name: String,        // what the picker shows: "Digital Rain"
    pub themes: Vec<Theme>,  // id + label, the second picker
    pub prefers_dark_cards: bool,
}
```

`id` and the theme's `id` are what cross at draw time. The name and the label
are for a person and never compared.

### 3.1 But it retires this morning's soname rule

The addon design chose **one soname per class** — a host accepting class `c`
asks the loader for `lib<c>.so.1` — because it is the smallest thing that
works and it makes two addons of one class collide rather than silently pick
one. That reasoning holds for a *reading source*, where one is what you want.

**It is wrong for backgrounds.** The whole point of the default pack being an
example is that somebody writes a second one, and two packs of class
`aura-background` both being installed is the case to support rather than to
refuse.

So this design **replaces the naming rule with a real enumeration channel**:
the supervisor sets an environment variable beside `LD_LIBRARY_PATH` naming
each class-matched library it joined, and the object inside it. It is composed
in `rivet_core::service`, next to the two functions that already compose the
path and the confinement window from one list — which is the arrangement that
stopped the loader and the kernel disagreeing earlier today.

The soname rule then becomes a convention rather than a mechanism, and a host
opens exactly what it was handed.

## 4. Drawing is a second call, and it is binary

A background draws at 30 fps into a 1920x1080 surface. Serializing a frame as
JSON is not a design to critique; it is arithmetic. So the protocol grows a
**second entry point** whose payload is a versioned `#[repr(C)]` descriptor:

```rust
#[repr(C)]
pub struct Frame {
    pub abi: u32,          // checked before anything else is read
    pub width: u32,
    pub height: u32,
    pub stride: u32,       // bytes per row, which is not width * 4
    pub format: u32,       // one enum value, XRGB8888 today
    pub pixels: *mut u8,   // the host's surface, valid for this call only
    pub time_ms: u64,      // the frame's own time, from the host's clock
    pub background: u32,   // an index into what Describe returned
    pub theme: u32,
}
```

**This is a deliberate exception to the protocol's first rule**, and it is
narrow enough to argue: `repr(C)` over fixed-width scalars and a pointer *is*
the C ABI, which is the one layout two separate Rust compilations do agree on.
The version is the first field and the version symbol still gates the load, so
a mismatched pack is refused before a pixel is written.

**The addon never allocates the surface and never keeps the pointer.** It
writes within `stride * height` bytes and returns; the host owns the buffer
before and after. That is the same rule the JSON boundary has, stated for
memory instead of for bytes.

**The cost.** Digital Rain measures 12.4 ms of the 33.3 ms budget today
(`SPEC-04` §10.7). Through an addon it is the same code writing the same
pixels, plus one indirect call per frame. What changes is not the frame budget
but where a fault lands: a background that scribbles past `stride * height`
corrupts the renderer, and the renderer is the process holding the display.
The addon-load trial is what bounds that — a pack that killed the renderer
last start is skipped and named.

## 5. What moves, and the rule for moving it

Digital Rain, Boxes and Solid move into `examples/aura/aura-background-pack`,
**constant for constant**, in the same sense the original port used: the C++
source was the authority then and the Rust in the renderer is the authority
now. Nothing is improved on the way across.

**The check that it worked**: the renderer's own tests already write frames to
`.ppm` for a person to look at. Capture a frame of each background and theme
from the renderer as it stands, move the code, capture again from the pack, and
compare the bytes. A background that renders one pixel differently has not been
ported.

## 6. Four new ones

Constrained by what Aura is: 1080p, dark, in a room where a person glances at
it, never competing with the cards in the corners. All four are cheap, and
three of them are cheap in the same way — compute small, scale up.

**Starfield.** Points drifting outward from a vanishing point in three parallax
layers, the near layer brighter and faster. A few hundred points, each a one-
or two-pixel plot, so the frame cost is a rounding error. Themes are the star
colour and the drift rate: white, amber, *Ice Blue* to match the existing
family.

**Aurora.** A plasma field — two or three summed sine functions over a
**low-resolution grid**, say 96x54, bilinearly scaled to the full surface. The
expensive-looking part is the gradient, and a gradient is the one thing scaling
does not hurt. Themes are the palette ramp: green-violet, deep blue, ember.

**Contours.** The same low-resolution height field, drawn as **iso-lines**
rather than filled: thin bright curves on black, drifting slowly. This is the
most legible of the four at night, because it is mostly black, and the one that
looks most like an instrument rather than a toy. Themes are the line colour and
the spacing between contours.

**Life.** Conway at roughly 160x90 cells with a long fade on dying cells, so
the pattern leaves a trail rather than flickering. It never repeats, it costs
one pass over 14,400 cells per step, and a step does not have to happen every
frame — four steps a second is livelier to watch than sixty. Themes are the
cell colour and the fade length.

**Two questions they raise together.** Whether a background may draw at less
than the frame rate and tell the host so (Life wants to), and whether the pack
or the host owns the *transition* when a person changes the picker. Neither
blocks a first version: draw every frame, and cut rather than fade.

## 7. Order

1. **The enumeration channel** (§3.1), in `rivet-core` and `rivet-sysd`, with
   the confinement window composed from the same list. It is small, it is
   platform work, and everything else waits on it.
2. **The frame call** (§4) in `rivetos-addon`, with a probe pair like the one
   that proved the JSON boundary — a pack that fills the surface with one
   colour, and a host that checks the bytes arrived.
3. **The pack**, with the three existing backgrounds moved and byte-compared.
4. **The renderer as a host**, and the controller as a describing host.
5. **The four new backgrounds**, which are then ordinary work in a repository
   that has an example to copy.

## 8. What is the maintainer's

* **Whether the pack ships in the Rivet repository.** It is the addon example
  as well as the default implementation, it carries no credential, and Rivet is
  going public — so `examples/aura/aura-background-pack` is the natural home
  and this design assumes it. Say otherwise and it becomes a repository of its
  own.
* **Whether a background may refuse a theme it does not know**, or must draw
  something. A refusal is honest; a black screen is a black screen.
* **The `repr(C)` exception in §4**, which is the one place this design spends
  a rule it was given.
