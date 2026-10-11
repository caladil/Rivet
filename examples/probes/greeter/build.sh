#!/bin/sh
# Build `greeter` for a Rivet and emit it as a signed application .rvt.
#
#   tools/container/exec.sh examples/probes/greeter/build.sh
#
# It needs `examples/probes/libgreet/build.sh` to have run, and it checks rather than
# assuming: the reference in `Cargo.toml` must be the reference of the library
# actually on disk. See "the check" below — it is the most important twelve
# lines in this file.
#
# ## Why the link line names the file rather than saying `-lgreet`
#
# `-lgreet` makes the linker look for `libgreet.so` — the unversioned
# *development* symlink a distribution ships in its `-dev` package. There is no
# such link here and there should not be: it exists so that a build can find
# "whatever the current libgreet is", and this whole mechanism is about naming
# exact bytes rather than whatever is current.
#
# Naming the file directly links against that file and records its `DT_SONAME`
# — `libgreet.so.1` — as the `DT_NEEDED`, which is what the loader will look
# for. Same result, one fewer thing to keep in step.
#
# There is deliberately **no `-rpath`** either. Where the library lives on a board
# is not a build-time fact: it is `/state/lib/<reference>/lib`, and the
# reference is a digest of the artifact, so an `-rpath` would either be wrong or
# would have to be patched in after signing — which is the one thing a signature
# exists to prevent. `rivet-sysd` composes `LD_LIBRARY_PATH` from the declared
# closure at start instead, so the resolution is the machine's and it is derived
# from something inside the signature.
#
# ## What the linker needs that the board does not have
#
# The link needs `libgreet.so.1` present as a FILE; the board needs it present
# as an installed artifact. They are the same bytes reached two different ways,
# which is ordinary for cross-compilation and worth saying because the two paths
# have nothing to do with each other.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
# **Three levels, not two** — see the same correction in
# `examples/probes/libgreet/build.sh` for what it cost and why nobody saw it.
# `REPO` was `examples/`, which made `LIBDIR` below
# `examples/examples/probes/libgreet/out` and the source line
# `examples/tools/toolchain/env.sh`. Neither exists.
REPO=$(cd "$HERE/../../.." && pwd)
LIBDIR=${GREET_OUT:-$REPO/examples/probes/libgreet/out}
OUT=${GREETER_OUT:-$HERE/out}

# shellcheck source=../../../tools/toolchain/env.sh
. "$REPO/tools/toolchain/env.sh"

[ -f "$LIBDIR/libgreet.so.1" ] || {
    echo "examples/probes/greeter/build.sh: no $LIBDIR/libgreet.so.1" >&2
    echo "  Run examples/probes/libgreet/build.sh first — this application links against it." >&2
    exit 1
}

mkdir -p "$OUT"

echo "--- compiling greeter for $RIVET_TARGET ---"
"$RIVET_CC" -O2 \
    -I "$REPO/examples/probes/libgreet/src" \
    -o "$OUT/greeter" \
    "$HERE/src/main.c" \
    "$LIBDIR/libgreet.so.1"
"$RIVET_STRIP" --strip-unneeded "$OUT/greeter"

echo "--- what it says it needs ---"
readelf -d "$OUT/greeter" | grep -E 'NEEDED' || true
readelf -l "$OUT/greeter" | grep -A1 INTERP | tail -1 || true

# --- the check -------------------------------------------------------------
#
# **The reference in Cargo.toml must be the library that is actually here.**
#
# Without this, the two could drift in the quietest possible way: the link would
# succeed against a rebuilt libgreet, the manifest would declare the digest of
# an older one, and the failure would arrive on a board — either as an install
# refusal naming a library nobody remembers, or, worse, as an application
# resolving to a library that is NOT the one it was linked against.
#
# `cargo rivet library` computes the reference from the artifact it wrote, so
# asking it again is the only honest way to know.
echo
echo "--- checking the declared closure against the library on disk ---"
cd "$REPO"
# **Newest, not first.** `examples/probes/libgreet/build.sh` writes `libgreet.so.1` and
# its `.rvt` in the same run, so the newest artifact is the one describing the
# object this build just linked against. Sorting by name would pick `1.0.0` out
# of a directory that also holds `2.0.0`, and would then refuse a build that is
# perfectly correct — which is worse than not checking, because it teaches
# whoever hits it to stop believing the check.
RVT=$(ls -t "$LIBDIR"/libgreet-*-aarch64-library.rvt 2>/dev/null | head -1)
[ -n "$RVT" ] || {
    echo "no library .rvt in $LIBDIR — run examples/probes/libgreet/build.sh" >&2
    exit 1
}
BUILT=$(cargo run --quiet --release -p cargo-rivet --bin cargo-rivet -- rivet inspect \
    "$RVT" --trusted-key "$REPO/keys/rivet-app.pub" 2>/dev/null \
    | sed -n 's/^reference *//p' | tr -d ' \r')
DECLARED=$(sed -n 's/^ *"\(libgreet-[0-9a-z.-]*\)",*$/\1/p' "$HERE/Cargo.toml" | head -1)

if [ "$BUILT" != "$DECLARED" ]; then
    cat >&2 <<EOF

REFUSED: examples/probes/greeter/Cargo.toml declares a library this build did not make.

  declared  $DECLARED
  built     $BUILT

Those are digests of ARTIFACTS, so a difference means the library changed —
a new version, edited source, a different compiler, or a different Buildroot
tree. Nothing here can tell which, and nothing here should guess.

Update the \`library = [...]\` line in examples/probes/greeter/Cargo.toml to the
reference above and build again. That is a deliberate act, which is the point:
an application's closure names bytes, and changing which bytes is a change to
what the application IS.
EOF
    exit 1
fi
echo "  ok — $DECLARED"

echo
echo "--- emitting the application .rvt ---"
cargo run --quiet --release -p cargo-rivet --bin cargo-rivet -- rivet package \
    --manifest-path "$HERE/Cargo.toml" \
    --binary "$OUT/greeter" \
    --out-dir "$OUT"
