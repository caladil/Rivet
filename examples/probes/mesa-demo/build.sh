#!/bin/sh
# Build `mesa-demo` — **the first dynamically-linked program in this project.**
#
#   tools/container/exec.sh examples/probes/mesa-demo/build.sh
#
# ## Why this is a script and not `cargo build`
#
# Four flags, none of them guessable, and every one of them was found by a
# linker error. They are recorded here rather than in somebody's shell history
# for `tools/buildroot/build.sh`'s reason: a build that lives in scrollback is a
# build nobody else can reproduce.
#
# ### `-C target-feature=-crt-static`
#
# **Rust's musl target links statically by default**, which is right for every
# other binary in this repository (`ADR-003` §5) and impossible here: there is
# no static `libEGL`. Without this the linker looks for `libgbm.a` and reports
# `unable to find library -lgbm`, which reads like a missing library and is
# actually a missing *flag*.
#
# ### `-C linker=…/aarch64-linux-gcc` and `-C link-self-contained=no`
#
# **`rust-lld` does not know where the toolchain's own libraries are.** With
# `-lgbm`, `-lEGL` and `-lGLESv2` resolved out of staging it went on to fail on
# `-lgcc_s`, and then on `-lgcc` — support libraries that live under
# `lib/gcc/<tuple>/<version>/`, a path the cross GCC computes and nothing else
# knows. Handing the link to GCC resolves all of them, because finding them is
# the job it exists to do.
#
# ### `-L …/staging/usr/lib`
#
# Where Buildroot puts Mesa's link-time libraries. The RUNTIME copies are on the
# card, in `/lib`, put there by `tools/card/build.sh`'s `collect_libraries`.
#
# ## What this produces, and why it can run at all
#
# A binary with `PT_INTERP = /lib/ld-musl-aarch64.so.1` and five `NEEDED`
# entries: `libgbm.so.1`, `libEGL.so.1`, `libGLESv2.so.2`, `libgcc_s.so.1` and
# `libc.so`. **Every one of them is on the card**, which is not luck — the
# loader is Shared Rivets' (`SPEC-02`) and the other four are the closure
# `collect_libraries` walks. A Rivet from before 2026-08-31 would refuse this
# binary at `execve` with `ENOENT` about the interpreter.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
BR=${RIVET_BR_OUTPUT:-/br}
TARGET=aarch64-unknown-linux-musl
GCC="$BR/host/bin/aarch64-linux-gcc"

[ -x "$GCC" ] || {
    echo "mesa-demo: no cross gcc at $GCC" >&2
    echo "    This has to run inside the build container:" >&2
    echo "    tools/container/exec.sh examples/probes/mesa-demo/build.sh" >&2
    exit 1
}
[ -e "$BR/staging/usr/lib/libEGL.so" ] || {
    echo "mesa-demo: no libEGL in $BR/staging/usr/lib — Mesa is not built." >&2
    echo "    BR2_PACKAGE_MESA3D in tools/buildroot/configs/rivet_aarch64_defconfig," >&2
    echo "    then tools/container/build-buildroot.sh --reconfigure." >&2
    exit 1
}

cd "$HERE"
RUSTFLAGS="-C target-feature=-crt-static \
 -C linker=$GCC \
 -C link-self-contained=no \
 -L $BR/staging/usr/lib" \
    cargo build --release --target "$TARGET"

OUT=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
BIN="$OUT/$TARGET/release/mesa-demo"

echo ""
echo "--- $BIN ---"
readelf -l "$BIN" | grep -o 'interpreter: [^]]*'
readelf -d "$BIN" | grep NEEDED | sed 's/.*\[/  needs /; s/\]//'
echo ""
echo "Every library above must be in /lib on the card. tools/card/build.sh's"
echo "collect_libraries walks that closure; a card built before it did prints"
echo "nothing at all here and refuses this binary at execve."
