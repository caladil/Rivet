# T6 background baseline — the "before" picture

* **Task:** T6, `docs/PLAN-Addon-Feature-Set-2026-09-19.md` — Digital Rain,
  Boxes and Solid move out of `examples/aura/aura-renderer` and into
  `examples/aura/aura-background-pack`, constant for constant.
* **Captured:** 2026-09-19, from `aura-renderer` 0.4.0, before any code moved.
* **Rule this exists to prove against:** *"A background that renders one
  pixel differently has not been ported"* (`SPEC-04` §12, T6's own entry).

**Why this `evidence/` and not the repository root's.** `.gitignore` line 104
already claims `/evidence/` at the repository root for HDMI captures of
hardware experiments — "kept beside the repo, never in it," i.e. deliberately
never committed. A committed baseline is the opposite of that, so it lives
here instead, inside the one area this task's brief permitted touching
(`examples/aura/aura-renderer/**`). The leading `/` on the root rule anchors
it to the repository root, so it does not reach this nested directory.

---

## What is in `manifest.json`

Every `(background, theme, frame index)` `aura-renderer` draws — all ten
backgrounds and themes `Background::name()` enumerates, at four fixed frame
indices each, 40 entries — with a BLAKE3 hash of that frame's raw canvas
bytes. No images are checked in: a hash is everything a later comparison
needs, and an image is large, regenerable, and the repository's `.gitignore`
already keeps build output (including the `.ppm` files this crate's other
tests write to `target/`) out of the tree.

Each entry:

```json
{
  "background_id": "digital-rain",
  "background_label": "Digital Rain",
  "theme_id": "phosphor-green",
  "theme_label": "Phosphor Green",
  "frame_index": 150,
  "frame_time_ms": 5000,
  "width": 1920,
  "height": 1080,
  "stride": 7680,
  "bytes_per_pixel": 4,
  "seed": 20260919,
  "hash_algorithm": "blake3",
  "hash": "09acbc00af5296023b05a1a81ba60f03fc65f5aedf0a42d980b71c0fa20db882"
}
```

`frame_index` is how many `update(1/30 s)` calls preceded the capture, not a
wall-clock time — see `examples/aura/aura-renderer/src/evidence.rs`'s module
doc comment for why counting frames rather than jumping to a large `dt`
matters here (Digital Rain's brightness decay is per **frame**, not per
second of elapsed time, so the two are not interchangeable). `frame_time_ms`
is carried alongside as the same number in a form a person can read without
doing the multiplication.

## The check

The manifest is generated and compared by
`examples/aura/aura-renderer/src/evidence.rs`, which renders these pixels
directly against the renderer's own drawing loop rather than diffing the JSON
— a JSON diff alone cannot tell you whether two hashes *should* match, only
whether they do. Run it from a shell with the build container:

```sh
tools/container/exec.sh sh -c 'cd /rivet/examples/aura/aura-renderer && cargo test evidence::'
```

Two tests run:

* `the_same_seed_and_frame_index_render_the_same_bytes_twice` — renders the
  whole manifest twice in one process and requires every hash to agree.
* `the_tree_still_renders_what_the_t6_baseline_recorded` — re-renders the
  manifest from whatever the tree currently does and compares it against
  this directory's `manifest.json`, **entry by entry**, reporting every
  `background_id`/`theme_id`/`frame_index` that differs (added, missing, or
  hashed differently) before failing. This is the "one command, told which
  entries differ" check.

Both pass against this tree today, because this baseline was captured from
this same code.

**After T6 moves the three backgrounds into `aura-background-pack`**, this
exact test stops applying — `aura-renderer` will no longer contain the code
that draws them. What survives is this manifest and its schema: the pack's
own tests render the same ten backgrounds and themes at the same four frame
indices from the same seed (`20260919`, `Mt19937::new`) and compare their
hashes against `manifest.json` by the same three keys. A background that
renders one pixel differently shows up there exactly the way it shows up
here — as a named, failing entry, not a guess.

## Regenerating

Not part of the check, and not run by a plain `cargo test`. Kept `#[ignore]`d
so a re-capture is a deliberate act:

```sh
tools/container/exec.sh sh -c \
  'cd /rivet/examples/aura/aura-renderer && \
   cargo test --bin aura-renderer evidence::tests::regenerate_the_t6_background_baseline_manifest -- --ignored'
```

## Proof this check actually catches something

Demonstrated 2026-09-19 by changing `Solid::colour`'s `Navy` value from
`[10, 18, 40]` to `[10, 18, 41]` — one unit in one channel — and running the
comparison test. It failed, naming exactly the four affected entries
(`solid/navy` at all four frame indices, correctly *not* naming any of the
other 36) with both hashes printed:

```
the T6 baseline comparison found 4 difference(s):
solid/navy @ frame 1 (33 ms): HASH DIFFERS — baseline b8956c63... now 3a3f6f0f...
solid/navy @ frame 30 (1000 ms): HASH DIFFERS — baseline b8956c63... now 3a3f6f0f...
solid/navy @ frame 150 (5000 ms): HASH DIFFERS — baseline b8956c63... now 3a3f6f0f...
solid/navy @ frame 600 (20000 ms): HASH DIFFERS — baseline b8956c63... now 3a3f6f0f...
```

The constant was then reverted and the check passes again — see the git
history of `examples/aura/aura-renderer/src/background.rs` around this
commit for the exact diff that was tried.

## Why four frame indices, and why these four

`1, 30, 150, 600` — roughly 33 ms, 1 s, 5 s and 20 s at the renderer's own
frame budget. Chosen to cover construction (frame 1, before any `update` has
moved anything), early motion, sustained motion, and — at 600 — the exact
boundary `background.rs`'s own
`most_columns_are_raining_at_any_given_moment` test already uses: "past
every start delay and past the first wrap for the faster columns." Reusing a
boundary this codebase already validated, rather than picking a fresh one,
is deliberate.

## What this does not prove

* **Boxes and Solid have never been captured off a board** (`SPEC-04` §12
  says so directly: "Boxes and Solid have unit tests and have not been on a
  screen"). This baseline proves the **pack renders the same bytes the
  renderer did**, not that either rendering is correct against the original
  C++ — that comparison is `SPEC-04` §12's, and remains open for those two.
* **The seed and frame indices are fixed**, so this baseline exercises one
  random sequence per background, not the full space `Mt19937` can produce.
  A port that broke only for a rare seed would not be caught here — the
  construction and update code is identical either way, so this is a
  reasonable bound to accept, not a gap that was missed.
