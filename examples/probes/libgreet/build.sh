#!/bin/sh
# Build `libgreet.so.1` for a Rivet, and emit it as a signed library .rvt.
#
# **This is the smallest complete example of Shared Rivets** and it is meant to
# be read before anything larger. Four things happen and each is one command:
#
#   1. the cross toolchain is found         tools/toolchain/env.sh
#   2. a shared object is built for musl    $RIVET_CC -shared -Wl,-soname,…
#   3. it becomes a signed artifact         cargo rivet library
#   4. the artifact prints its REFERENCE    which an application declares
#
# Run it in the container, where the toolchain is:
#
#   tools/container/exec.sh examples/probes/libgreet/build.sh
#
# ## The `-Wl,-soname` is not optional and not decoration
#
# A loader finds a library BY its SONAME: an application's `DT_NEEDED` holds a
# name and the loader opens a file called that. Without `-soname`, GCC records
# no `DT_SONAME`, `cargo rivet library` refuses the object, and the refusal says
# this. That refusal is the point — an object with no SONAME would install
# perfectly, sit in the right directory, and never be found.
#
# ## Why the version is compiled IN
#
# `GREET_VERSION` reaches the source, so two builds that differ only in the
# version produce different bytes, therefore different digests, therefore
# different install directories. That is the property to demonstrate: a board
# can hold both at once and each application resolves to the one it declared.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
# **Three levels, not two** — `examples/probes/libgreet` — and it was two until
# 2026-09-04. `examples/` grew a `probes/` layer on 2026-09-01 (`ADR-036` §6.1,
# "the directory says which is which") and this line moved with the files
# without being corrected, so `REPO` was `examples/` and the very next line
# sourced `examples/tools/toolchain/env.sh`, which does not exist.
#
# **The script has therefore not run since the reorganisation**, and nobody
# noticed because its output was committed. That is the second half of the same
# finding: a tracked build product is a build recipe nobody has to run.
REPO=$(cd "$HERE/../../.." && pwd)
VERSION=${GREET_VERSION:-1.0.0}
OUT=${GREET_OUT:-$HERE/out}

# shellcheck source=../../../tools/toolchain/env.sh
. "$REPO/tools/toolchain/env.sh"

mkdir -p "$OUT"

echo "--- compiling libgreet.so.1 ($VERSION) for $RIVET_TARGET ---"
# `-fPIC` because a shared object is mapped at an address chosen at load time.
# `-O2` because there is no reason to ship an unoptimised library and because a
# build flag nobody set is a build flag nobody can reproduce.
"$RIVET_CC" \
    -shared -fPIC -O2 \
    -DGREET_VERSION="\"$VERSION\"" \
    -Wl,-soname,libgreet.so.1 \
    -o "$OUT/libgreet.so.1" \
    "$HERE/src/greet.c"

"$RIVET_STRIP" --strip-unneeded "$OUT/libgreet.so.1"

echo "--- what it says about itself ---"
"$RIVET_CC" -print-sysroot >/dev/null   # the toolchain is real; already checked
readelf -d "$OUT/libgreet.so.1" | grep -E 'SONAME|NEEDED' || true

echo
echo "--- emitting the library .rvt ---"
cd "$REPO"
cargo run --quiet --release -p cargo-rivet --bin cargo-rivet -- rivet library \
    --name libgreet \
    --version "$VERSION" \
    --object "$OUT/libgreet.so.1" \
    --out-dir "$OUT"
