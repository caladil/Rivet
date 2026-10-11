#!/bin/sh
# ~~Build Aura's renderer for a Rivet~~ — **RETIRED 2026-09-04.**
#
# **This refuses to run.** Ruled by the maintainer: the C++ renderer is retired.
# `ADR-036` §6.2B had already ruled the Rust renderer the one that ships, and
# `.rulings/` R-138 made this one unbuildable in practice — Raylib's DRM backend
# opens `/dev/dri/card0` by path and a lessee is no longer granted that node, so
# a build of this would package, install, start and draw nothing.
#
# **Kept rather than deleted, and the refusal is why.** `CMakeLists.txt` beside
# this file is the only written record of how Raylib's DRM backend was
# cross-compiled against the Buildroot sysroot — an external system's shape,
# which `AGENTS.md` §2 rule 3 says is the thing a dormant file is worth keeping
# for. Deleting the recipe would throw that away to save nothing; leaving it
# runnable would let somebody spend an afternoon on a binary that cannot take
# the screen.
#
# The Rust renderer is `examples/aura/aura-renderer`. It takes a display LEASE
# and is what has been drawing on the maintainer's board.
#
# ~~   RIVET_AURA_SRC=<your Aura checkout> \
#      tools/container/exec.sh deploy/aura/renderer/build.sh~~
echo "deploy/aura/renderer: RETIRED 2026-09-04 and this build refuses." >&2
echo "" >&2
echo "  The C++/Raylib renderer opens /dev/dri/card0 by path. Since .rulings/" >&2
echo "  R-138 an application is granted the render node only, so this binary" >&2
echo "  would package, install, start, and draw nothing at all." >&2
echo "" >&2
echo "  What ships is examples/aura/aura-renderer, which takes a display lease." >&2
echo "  ADR-036 §6.2B ruled that; PLAN.md §119 is the port." >&2
echo "" >&2
echo "  CMakeLists.txt beside this file is kept as the record of how Raylib's" >&2
echo "  DRM backend was cross-compiled. Reviving this means teaching Aura's C++" >&2
echo "  to accept a handed-over descriptor, in Aura's own repository." >&2
exit 1

# ---------------------------------------------------------------------------
# Everything below is the recipe as it stood, unreachable and kept for the
# reason above.
# ---------------------------------------------------------------------------
#
# Aura's own tree is mounted READ-ONLY at /aura and nothing here writes to it.
# The CMakeLists beside this file carries why the build differs from Aura's:
# Raylib's DRM backend instead of GLFW-on-Wayland-under-Cage, because a Rivet
# has no compositor and is not going to have one.
#
# ## Why this binary is DYNAMIC, when almost nothing else on a Rivet is
#
# `ADR-003` §5 makes every Rivet binary static, and this one cannot be: Mesa
# ships shared objects and there is no static `libEGL` (`PLAN.md` §104.17). So
# this is the second dynamically-linked program in the project, after
# `examples/probes/mesa-demo`, and it needs the same five things on the card —
# the loader plus `libEGL`, `libGLESv2`, `libgbm` and `libgcc_s`, which
# `PUBLIC_LIBRARIES` promises and `tools/card/build.sh` installs.
#
# ## The `-static` this has to undo, and why it is not a bug in Buildroot
#
# `/br/host/share/buildroot/toolchainfile.cmake` sets `-static` into
# `CMAKE_C_FLAGS`, `CMAKE_CXX_FLAGS` and every linker-flags variable, as CACHE
# entries. That is correct for this project: `ADR-003` §5 is the rule and a
# toolchain that defaulted the other way would let a static-by-policy build
# quietly produce a dynamic binary.
#
# The failure without the override is worth quoting, because it names a file
# and not the flag:
#
#   ld: attempted static link of dynamic object `…/usr/lib/libGLESv2.so'
#
# So the overrides below are deliberate and narrow, and they are passed on the
# command line rather than written into the toolchain file — this is one
# program that must be dynamic, not a change of policy.
#
# ## `--as-needed`, which removes a question rather than answering it
#
# Raylib links `-latomic` unconditionally. Nothing in this binary references it
# on aarch64, so without `--as-needed` it lands in NEEDED and the platform is
# asked to promise a library nobody uses. With it, the link drops what is not
# referenced and the eight NEEDED entries become seven — every one of them
# something `PUBLIC_LIBRARIES` already promises.
#
# **It is safe for Mesa's dlopened driver.** `--as-needed` drops only libraries
# this binary names and does not reference; `libgallium` is opened by `libEGL`
# at run time and was never on this link line.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
AURA=${RIVET_AURA_SRC:-/aura}
BUILD=${RIVET_RENDERER_BUILD:-/tmp/aura-renderer}
TOOLCHAIN=/br/host/share/buildroot/toolchainfile.cmake

[ -f "$AURA/cpp/src/main.cpp" ] || {
    echo "aura-renderer: no Aura sources at $AURA" >&2
    echo "    Run this inside the build container, which mounts them:" >&2
    echo "    RIVET_AURA_SRC=<path to Aura> tools/container/exec.sh $0" >&2
    exit 1
}
[ -f "$TOOLCHAIN" ] || {
    echo "aura-renderer: no Buildroot toolchain at $TOOLCHAIN" >&2
    echo "    This has to run in the build container (ADR-026 §7)." >&2
    exit 1
}

export PATH=/br/host/bin:$PATH

# Buildroot's own flags, minus `-static`. Kept otherwise byte-for-byte so this
# build differs from every other target build in exactly one way.
COMMON="-D_LARGEFILE_SOURCE -D_LARGEFILE64_SOURCE -D_FILE_OFFSET_BITS=64 -O2 -g0"

cmake -S "$HERE" -B "$BUILD" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN" \
    -DAURA_SOURCE_DIR="$AURA" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_FLAGS="$COMMON" \
    -DCMAKE_CXX_FLAGS="$COMMON" \
    -DCMAKE_EXE_LINKER_FLAGS="-Wl,--as-needed" \
    -DCMAKE_SHARED_LINKER_FLAGS="" \
    -DCMAKE_MODULE_LINKER_FLAGS=""

cmake --build "$BUILD"

BIN="$BUILD/aura-renderer"
echo ""
echo "--- $BIN ---"
readelf -l "$BIN" | grep -o 'interpreter: [^]]*' || echo "  STATIC — which should not happen; see the note above"
readelf -d "$BIN" | grep NEEDED | sed 's/.*\[/  needs /; s/\]//'
echo ""
# **The list is READ, not restated.** A copy of it here would be right today and
# wrong the next time the platform promises something — which it has done twice
# in one day. `rivet-core` is the authority and this prints what it says.
PROMISED=$(sed -n '/^pub const PUBLIC_LIBRARIES/,/^\];/p' \
    /rivet/rivet-core/src/package/library.rs 2>/dev/null \
    | sed -n 's/.*"\(lib[^"]*\)".*/\1/p' | tr '\n' ' ')
if [ -n "$PROMISED" ]; then
    echo "The platform promises: $PROMISED"
    echo "Anything NEEDED above and not in that list has to arrive as its own signed"
    echo "library .rvt, named in this application's closure (SPEC-02)."
else
    echo "Could not read PUBLIC_LIBRARIES from rivet-core; compare the list by hand."
fi
