# Image content

Files here are compiled or staged into the system image and are shared by more
than one program. Each one says why it is committed rather than fetched.

* `fonts/` — the system typeface and the icon face. `fonts/README.md`.
* `shield-512.rgba` — the Rivet mark, 512×512 raw RGBA, premultiplied by
  nothing: four bytes per pixel in `R G B A` order, 1 MiB, `include_bytes!` at
  compile time.

  **Raw rather than PNG**, and that is the whole reason it is a megabyte: a
  decoder would be a dependency in the two programs that draw it, for one image
  that never changes. It is `include_bytes!`'d, so it costs nothing at run time
  and the linker drops nothing.

  Shared by `cmd/rivet-splash` and `cmd/rivet-launcher` since `PLAN.md` §104.44
  — the launcher shows it on a controlled shutdown, and a second copy would be
  two marks that could come to differ.
