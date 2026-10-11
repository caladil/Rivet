#!/bin/sh
# The cross toolchain, exposed. `ADR-003` §6.3, Phase A.
#
# Source this and you can build C, C++ and Rust for a Rivet:
#
#   . tools/toolchain/env.sh
#   "$RIVET_CC" -O2 -shared -Wl,-soname,libfoo.so.1 -o libfoo.so.1 foo.c
#   cargo build --release --target "$RIVET_TARGET"
#
# ## Why this file exists
#
# On 2026-08-30 two agents from different vendors, with no shared context, were
# each told to put a database-backed application on a Rivet board. Both rejected
# SQLite. Both gave the same reason, unprompted: there is no C cross-toolchain
# they could find, so a crate that compiles C could not be built for the target.
#
# **A C cross-toolchain was already here.** Buildroot produces one, the card
# build has used it since 2026-08-28, and it was never exposed to anyone
# building an application. This file is that exposure, and it is plumbing rather
# than design — no linking decision changes, no loader ships because of it, and
# `ADR-003` §3's guarantees are untouched.
#
# See `docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §3.
#
# ## What it sets, and why the names are what they are
#
#   CC_aarch64_unknown_linux_musl   the `cc` crate's spelling. A Rust build
#   CXX_aarch64_unknown_linux_musl  script compiling C or C++ reads exactly
#   AR_aarch64_unknown_linux_musl   these, so a `-sys` crate builds with no
#                                     further configuration
#   RIVET_CC / RIVET_CXX / RIVET_AR / RIVET_STRIP / RIVET_SYSROOT / RIVET_TARGET
#                                     the same tools under names a human types
#
# The two sets are the same programs. The first set is what Cargo's build
# scripts look for and nobody would guess; the second is what a person writing
# a build script wants and would otherwise have to derive.
#
# ## It refuses rather than guessing
#
# A `$BR_OUTPUT` with no compiler in it produces a refusal that names the path
# it looked at. The alternative is an environment that is *set* and *wrong*,
# which fails later, in a build script, as a linker error about a file nobody
# named — and both agents above spent time on exactly that class of failure.
#
# ## The sysroot is ASKED FOR, never composed
#
# `BR2_TOOLCHAIN_EXTERNAL_BOOTLIN` puts the real toolchain under
# `host/opt/ext-toolchain` and leaves symlinks in `host/bin`, so a hard-coded
# `host/<tuple>/sysroot` is one configuration change away from being wrong.
# `gcc -print-sysroot` is the compiler's own answer and cannot disagree with it.

# Not `set -e`: this file is SOURCED, and an `exit` here would close the
# caller's shell. Every failure below returns instead.

RIVET_TARGET=${RIVET_TARGET:-aarch64-unknown-linux-musl}

_rivet_br=${RIVET_BR_OUTPUT:-}
if [ -z "$_rivet_br" ]; then
    if [ -d /br/host/bin ]; then
        # Inside tools/container: the Buildroot tree is mounted read-only at /br.
        _rivet_br=/br
    else
        _rivet_br="$HOME/.rivet/buildroot-2026.02.3"
    fi
fi

_rivet_prefix="$_rivet_br/host/bin/aarch64-linux"

if [ ! -x "$_rivet_prefix-gcc" ]; then
    echo "tools/toolchain/env.sh: no cross compiler at $_rivet_prefix-gcc" >&2
    echo >&2
    echo "  RIVET_BR_OUTPUT names the Buildroot output tree; it was taken as" >&2
    echo "  $_rivet_br. Inside tools/container/* the tree is mounted at /br and" >&2
    echo "  this file finds it without being told." >&2
    echo >&2
    echo "  Build one with tools/container/build-buildroot.sh. It takes hours" >&2
    echo "  and is a separate question — see tools/container/Dockerfile." >&2
    echo >&2
    echo "  NOTHING WAS SET. A half-set environment is worse than none: it" >&2
    echo "  fails later, inside somebody's build script, as an error about a" >&2
    echo "  file nobody named." >&2
    return 1 2>/dev/null || exit 1
fi

RIVET_CC="$_rivet_prefix-gcc"
RIVET_CXX="$_rivet_prefix-g++"
RIVET_AR="$_rivet_prefix-ar"
RIVET_STRIP="$_rivet_prefix-strip"
RIVET_SYSROOT=$("$RIVET_CC" -print-sysroot)

if [ -z "$RIVET_SYSROOT" ] || [ ! -d "$RIVET_SYSROOT" ]; then
    echo "tools/toolchain/env.sh: $RIVET_CC reports no usable sysroot" >&2
    echo "  It said: ${RIVET_SYSROOT:-<nothing>}" >&2
    return 1 2>/dev/null || exit 1
fi

# The `cc` crate's names. A Rust build script compiling C for the target reads
# these and needs nothing else.
CC_aarch64_unknown_linux_musl="$RIVET_CC"
CXX_aarch64_unknown_linux_musl="$RIVET_CXX"
AR_aarch64_unknown_linux_musl="$RIVET_AR"

export RIVET_TARGET RIVET_CC RIVET_CXX RIVET_AR RIVET_STRIP RIVET_SYSROOT
export CC_aarch64_unknown_linux_musl
export CXX_aarch64_unknown_linux_musl
export AR_aarch64_unknown_linux_musl

if [ -z "${RIVET_TOOLCHAIN_QUIET:-}" ]; then
    echo "--- Rivet cross toolchain ---"
    echo "  target   $RIVET_TARGET"
    echo "  cc       $RIVET_CC"
    echo "  c++      $RIVET_CXX"
    echo "  sysroot  $RIVET_SYSROOT"
    "$RIVET_CC" --version | head -1 | sed 's/^/  gcc      /'
fi

unset _rivet_br _rivet_prefix
