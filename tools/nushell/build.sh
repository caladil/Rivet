#!/bin/sh
# Cross-compile `nu` for Rivet — ADR-005 §5.1's interactive login shell.
#
# `ADR-005` §5.1 ruled nushell the interactive login shell on 2026-08-22, §5.6
# proved it builds and runs, and the ruling made it `root`'s shell on 2026-08-24.
# This script is the recipe that produces the binary `cmd/rivet-image
# --nushell` takes, and it exists for the reason `tools/buildroot/build.sh`
# gives for itself: the build must be a repository artefact rather than one
# person's shell history.
#
# Usage:
#   ./build.sh [--version X.Y.Z] [--no-default-features] [--verify-only PATH]
#
# Options:
#   --version X.Y.Z        which nushell to build. Default: the pin below.
#   --no-default-features  the trimmed build — ADR-005 §5.6.2 measures it at
#                          about 29 % smaller, at the cost of SQLite, HTTP/TLS,
#                          the LSP, the MCP server and the helix edit mode.
#                          NOT the default: nothing has ruled the trim in, and
#                          §5.6.2 records the saving rather than recommending it.
#   --verify-only PATH     skip the build and run §"What is verified" against an
#                          existing binary. What CI would call, and what to use
#                          when checking a binary somebody handed you.
#
# Environment:
#   RIVET_BR_OUTPUT   Buildroot's output tree, for the cross-toolchain.
#                       Default: $HOME/.rivet/buildroot-<pin>, matching
#                       tools/buildroot/build.sh's own default.
#   RIVET_NU_OUTPUT   where to build and install. Default: $HOME/.rivet/nushell
#
# ## Why a *Rust* program needs a C cross-toolchain
#
# It should not, and `.cargo/config.toml` says so: Rivet's own binaries link
# with `rust-lld` and need no C toolchain at all. nushell is the exception and
# `ADR-005` §5.6.3 established it by building: `fff-search` pulls `libgit2-sys`
# (with `libz-sys`), `lmdb-master-sys` and `blake3`; `nu-command`'s `os` feature
# hard-includes `fff-search`; and `os` is also what brings `crossterm`,
# `reedline` and `which`, so it cannot be dropped. **There is no feature
# combination that yields a pure-Rust `nu`** — verified by putting the cross
# `gcc` off `PATH` and watching `--no-default-features` fail too.
#
# ## Which toolchain, and why not the one the survey used
#
# **Buildroot's**, from `tools/buildroot/build.sh`'s output tree.
#
# The 2026-08-23 survey used a musl.cc tarball downloaded into a scratch
# directory, which was right for a survey and would be wrong here. `ADR-009` §6
# ruled Buildroot in for **the musl cross-toolchain and the C mechanisms**; a
# second musl toolchain, pinned differently and updated by nobody, is exactly
# the second authority `ADR-011` is about — and the one thing it would be
# authoritative for is the libc that ends up statically linked inside the
# operator's login shell.
#
# So this script refuses rather than downloading a toolchain. If Buildroot has
# not been built, `tools/buildroot/build.sh --fetch toolchain` is the answer,
# and it is the same answer every other vendored C mechanism already needs.
#
# ## What is verified, and why the script refuses rather than warns
#
# `ADR-003` §3's three checkable properties, read off the produced ELF:
#
#   1. `file` reports *statically linked*
#   2. `readelf -l` shows no `PT_INTERP` — no dynamic loader is requested
#   3. `readelf -d` reports no dynamic section
#
# A dynamically linked `nu` would build cleanly, install cleanly, and fail at
# `exec` on the board with `No such file or directory` naming the *interpreter*
# rather than the binary — the failure `ADR-003` §4 chose musl to avoid and the
# one that is hardest to read from a console. `AGENTS.md` §5: refuse rather than
# guess, and never report configured as working.
#
# The size is printed rather than checked. `ADR-005` §5.6.2 has the number this
# moves — `ADR-018` §10's distributed artifact, roughly +42 % — and a threshold
# invented here would be a constraint nobody ruled.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)

# ---------------------------------------------------------------------------
# The pin
# ---------------------------------------------------------------------------

# 0.115.1 is the version `ADR-005` §5.6.1 actually built and ran: statically,
# with default features, on the first attempt, with no patches and no feature
# trimming, and then under `qemu-system-aarch64` on Rivet's own kernel.
#
# Pinned rather than tracking latest, on `ADR-009` §3's terms: a version is a
# claim about what was verified, and "whatever crates.io had that morning" is
# not one. Moving the pin means re-reading §5.6's table, not editing this line.
NU_VERSION=${NU_VERSION:-0.115.1}

# Must match `tools/buildroot/build.sh`. Two literals for one fact, and the
# check below is what stops them drifting silently.
BR_VERSION=2026.02.3

FEATURES=""
VERIFY_ONLY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            shift
            [ $# -gt 0 ] || { echo "--version needs a value" >&2; exit 2; }
            NU_VERSION=$1
            ;;
        --no-default-features)
            FEATURES="--no-default-features"
            ;;
        --verify-only)
            shift
            [ $# -gt 0 ] || { echo "--verify-only needs a path" >&2; exit 2; }
            VERIFY_ONLY=$1
            ;;
        -h|--help)
            sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option $1" >&2
            exit 2
            ;;
    esac
    shift
done

TARGET=aarch64-unknown-linux-musl
NU_OUTPUT=${RIVET_NU_OUTPUT:-$HOME/.rivet/nushell}
BR_OUTPUT=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-$BR_VERSION}

# ---------------------------------------------------------------------------
# Verify — the whole of what this script promises about a binary
# ---------------------------------------------------------------------------

verify() {
    bin=$1
    readelf=$2

    if [ ! -f "$bin" ]; then
        echo "verify: $bin does not exist" >&2
        return 1
    fi

    echo
    echo "--- ADR-003 §3, read off $bin ---"

    # 1. `file`, when it is there. Not fatal on its own: the two `readelf`
    #    checks below are the load-bearing ones and `file` is the readable
    #    summary of them. Saying which check was skipped is the point —
    #    `AGENTS.md` §7, an absent tool and a passing check must not look alike.
    if command -v file >/dev/null 2>&1; then
        printf '  file:         '
        file -b "$bin"
        if ! file -b "$bin" | grep -q "statically linked"; then
            echo "  REFUSED: file(1) does not call this statically linked" >&2
            return 1
        fi
    else
        echo "  file:         (file(1) is not installed; the two readelf checks below stand alone)"
    fi

    # 2. No PT_INTERP. This is the one that decides whether the binary runs on
    #    a Rivet board at all: the image ships no dynamic loader, so a
    #    requested interpreter is a request nothing can answer.
    if "$readelf" -lW "$bin" 2>/dev/null | grep -qi "interpreter"; then
        echo "  PT_INTERP:    PRESENT — REFUSED" >&2
        "$readelf" -lW "$bin" | grep -i interpreter >&2
        return 1
    fi
    echo "  PT_INTERP:    absent (no dynamic loader requested)"

    # 3. No dynamic section.
    if "$readelf" -dW "$bin" 2>/dev/null | grep -q "Dynamic section at offset"; then
        echo "  .dynamic:     PRESENT — REFUSED" >&2
        return 1
    fi
    echo "  .dynamic:     absent"

    printf '  machine:      '
    "$readelf" -hW "$bin" | sed -n 's/.*Machine: *//p'
    printf '  size:         '
    stat -c '%s bytes' "$bin"
    echo
    echo "  ADR-005 §5.6.2 has what this costs the distributed artifact."
    return 0
}

# ---------------------------------------------------------------------------
# The toolchain — named explicitly, and refused rather than substituted
# ---------------------------------------------------------------------------

TOOL_PREFIX="$BR_OUTPUT/host/bin/aarch64-linux"
READELF="$TOOL_PREFIX-readelf"
[ -x "$READELF" ] || READELF=readelf

if [ -n "$VERIFY_ONLY" ]; then
    verify "$VERIFY_ONLY" "$READELF"
    exit $?
fi

if [ ! -x "$TOOL_PREFIX-gcc" ]; then
    cat >&2 <<EOF
tools/nushell/build.sh: no cross-toolchain at $TOOL_PREFIX-gcc

nushell is a Rust program that nonetheless needs a C cross-toolchain — see
"Why a *Rust* program needs a C cross-toolchain" at the top of this script,
and ADR-005 §5.6.3 for how that was established.

The toolchain is Buildroot's, by ADR-009 §6. Build it:

    tools/buildroot/build.sh --fetch toolchain

or point RIVET_BR_OUTPUT at an existing output tree. This script does not
download a toolchain of its own: a second musl, pinned differently, would be
the libc statically linked inside the operator's login shell and nothing would
own it (ADR-011).
EOF
    exit 1
fi

# What the toolchain says it is, rather than what its path implies. A prefix
# named `aarch64-linux` proves nothing about the libc, and the libc is the
# whole question — ADR-003 §4 chose musl, and a static
# `getpwnam` depends on that choice.
TRIPLE=$("$TOOL_PREFIX-gcc" -dumpmachine)
case "$TRIPLE" in
    *-musl)
        ;;
    *)
        echo "tools/nushell/build.sh: $TOOL_PREFIX-gcc reports $TRIPLE, which is not a musl" >&2
        echo "toolchain. ADR-003 §4 is not optional here; refusing rather than linking against" >&2
        echo "whatever this is." >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

mkdir -p "$NU_OUTPUT"

# `cargo install` rather than a git clone, so the source is the published
# crate and `--locked` makes the published Cargo.lock the one that builds.
# A clone would need a tag, a fetch, and a second answer to "which source".
#
# The three `*_aarch64_unknown_linux_musl` variables are the `cc` crate's
# interface and are what actually route libgit2, LMDB, zlib and blake3 through
# the cross-toolchain. The linker variable is Rust's own and is set explicitly:
# the repository's `.cargo/config.toml` names `rust-lld` for this target, which
# is correct for Rivet's own pure-Rust binaries and is not what should link a
# binary carrying four C libraries built by this gcc.
export CC_aarch64_unknown_linux_musl="$TOOL_PREFIX-gcc"
export CXX_aarch64_unknown_linux_musl="$TOOL_PREFIX-g++"
export AR_aarch64_unknown_linux_musl="$TOOL_PREFIX-ar"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$TOOL_PREFIX-gcc"

echo "nushell:    $NU_VERSION"
echo "target:     $TARGET"
echo "toolchain:  $TOOL_PREFIX-gcc ($TRIPLE)"
echo "output:     $NU_OUTPUT"
echo "features:   ${FEATURES:-default}"
echo

# `--root` puts the binary at $NU_OUTPUT/bin/nu. `--target-dir` keeps the
# intermediate tree out of $HOME/.cargo so a second consumer of this script
# does not share 6 GB of nushell object files with Rivet's own builds.
#
# NOTE the deliberate `cd /`: run from inside the repository, cargo would read
# Rivet's `.cargo/config.toml` and its `rust-lld` linker would override the
# variable above. This is not hypothetical — that config exists precisely
# because Rivet's own binaries want the opposite setting.
cd /
cargo install nu \
    --version "$NU_VERSION" \
    --locked \
    --target "$TARGET" \
    --root "$NU_OUTPUT" \
    --target-dir "$NU_OUTPUT/target" \
    $FEATURES

BIN="$NU_OUTPUT/bin/nu"

# Strip, and say what it cost.
#
# `cargo install` leaves the symbols in — 75 056 336 bytes for 0.115.1, against
# the **42.4 MiB** `ADR-005` §5.6.2 tabulates. That table is the number the
# maintainer ruled on, so a recipe that shipped the unstripped binary would put
# 32 MB per slot on the card that nobody agreed to and that no reader of §5.6.2
# would expect.
#
# Done here rather than through `profile.release.strip`, deliberately: this is
# somebody else's crate and its profile is its own business, and a `--config`
# override would silently invalidate the whole build cache on every run.
# Buildroot's `strip` is used because it is the toolchain that produced the
# objects — `strip` is architecture-specific, and the host's would refuse.
STRIP="$TOOL_PREFIX-strip"
if [ -x "$STRIP" ]; then
    before=$(stat -c '%s' "$BIN")
    "$STRIP" "$BIN"
    after=$(stat -c '%s' "$BIN")
    echo
    echo "stripped:   $before -> $after bytes (ADR-005 §5.6.2 tabulates 42.4 MiB)"
else
    echo
    echo "NOT STRIPPED: no $STRIP. The binary carries its symbols and is roughly" >&2
    echo "75 MB rather than the 42.4 MiB ADR-005 §5.6.2 tabulates — on both slots." >&2
fi

verify "$BIN" "$READELF"

cat <<EOF

The binary is $BIN. It is NOT committed and must not be — ADR-010 §6.1, and
the same rule the kernel and the firmware are under.

Put it on a card:

    cargo run -p rivetos-image -- out/card.img \\
        <init> <supervisor> <cli> <netd> <mcpd> <timed> \\
        --nushell $BIN

Without --nushell the card's login shell is cmd/rivet, which is what every
card built before 2026-08-24 had and which the ruling that makes nu optional keeps working
on purpose. $HERE/README.md has the rest.
EOF
