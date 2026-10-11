# The build container's image tag, in ONE place.

`rivet-card-build:<this>` is the box `build-card.sh`, `build-update.sh`,
`qemu.sh` and `exec.sh` all run in. The number lived in **four** copies until
2026-09-02 — three `IMAGE_VERSION=2` assignments and one hard-coded
`IMAGE=rivet-card-build:2` in `exec.sh`.

Four copies of one fact is `ADR-011`'s problem in the tooling, and it had a
concrete failure mode waiting: a bump that updated three of them would leave one
script silently running the OLD image, building against a different toolchain
than everything else, and reporting success.

**Bump this file when the `Dockerfile` changes in a way a stale image would get
wrong** — a new component, a new package, a new toolchain pin. The scripts
rebuild automatically when the tag names an image that does not exist locally.

| version | what changed |
| :-- | :-- |
| 1 | the original card-build box |
| 2 | `qemu-system-aarch64` and `cpio`, 2026-08-30 |
| 3 | **`clippy`**, 2026-09-02 — see the `Dockerfile`'s note. Without it `cargo clippy` in this container printed nothing and exited non-zero, so a lint run here read as clean while three errors and 287 warnings stood |
| 4 | **`rustfmt` and host `libdrm`**, 2026-09-03, and the component shims copied to `/usr/local/bin` beside `cargo`. `tools/test-all.sh` wanted `cargo fmt` and got *"'cargo-fmt' is not installed for the toolchain"* — version 3's lesson at one component's remove — and it could not link any consumer of the SDK's `screen` feature, which needs `-ldrm` for the host as well as for a card. The shim copies matter as much as the component: an existing `rivet-card-cargo` volume is **not** reseeded by a newer image, so without them a machine that already had the project set up would still have no `cargo-fmt` on `PATH` |
| 5 | **`dbus`**, 2026-09-06 — INCOMPLETE, superseded by 6 the same hour, for an SDK ADOPTER rather than for Rivet. `exec.sh`'s `RIVET_APP_SRC` makes this container the Linux environment an adopter is verified in, and TempMon's `cargo test --workspace` died at `libdbus-sys` — the build script `btleplug` drags in on Linux — before compiling a line. `PLAN.md` §146.4 makes a Linux build a precondition of publishing, and a precondition that cannot be evaluated in the only Linux this project has is not one. Nothing on a card links D-Bus; Rivet has no message bus |
| 7 | **`mesa`**, 2026-09-21 — host `libEGL`, `libGLESv2` and `libgbm`, which Arch ships in one package. Version 4's lesson one feature later: the SDK's `composited-gpu` links all three, so `cargo test --workspace --all-features` could not LINK here (*"unable to find library -lgbm"*) and had been failing silently long enough that it read as a known limitation. It stopped being tolerable when `aura-renderer` took that feature to draw GPU backgrounds (R-206) and its own 81 tests stopped building. **Nothing here can RUN a GPU test** — no render node, and `gpu::take()` says so by name — this only makes the code compile and its unit tests link |
| 8 | **`libxkbcommon`**, 2026-09-23 — host copy, for `rivet-displayd`'s tests. Smithay links `-lxkbcommon` unconditionally, so from the day the compositor was built on it the compositor's host test binary could not LINK here and `tools/test-all.sh` reported it failing; five other failures stacked on the same red result, so nobody read it. Cards were never affected — they link Buildroot's aarch64 library |
| 9 | **`cargo-deny 0.20.2`**, 2026-09-25 — the dependency gate, `cargo install`ed and pinned with `--locked`, plus its shim copied to `/usr/local/bin` for the same volume-shadowing reason as `rustfmt` at version 4. Before this, `tools/test-all.sh` ran the gate only on whichever workstation happened to have the binary installed by hand, and reported "NOT CHECKED HERE" on every run made in this container |
| 6 | **`pkgconf`**, 2026-09-06, the other half of 5. Adding `dbus` alone changed nothing: Arch ships `dbus-1.pc` in that package and ships no `pkg-config` binary in this image at all, and `libdbus-sys` finds the library only through `pkg-config`. The build after version 5 failed with the identical error, which is the useful part of the story — the package that was missing was not the one the error message named |
| 10 | **pacman's `DisableSandboxSyscalls`**, 2026-09-28 (`.rulings/` R-226, `ADR-062` §6). Rosetta has no seccomp, so on an Apple Silicon Mac every package step refused and neither image could be built. The finished image is unchanged; the tag moves so no machine reuses an image built from the old recipe. `rivet-buildroot` went from 1 to 2 for the same change |
| 11 | **Rust 1.99.0**, from 1.97.1, matching `rust-toolchain.toml`. 1.98.0 was skipped for a trait-object vtable miscompile fixed in 1.98.1. An image built from the old recipe would compile with the old toolchain and report success, so the tag moves with the pin |
