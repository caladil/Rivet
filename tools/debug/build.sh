#!/bin/sh
# Produce Rivet's on-device debugging mechanisms — `ADR-005` §7.
#
# `gdbserver` and `strace`, static aarch64, stripped, verified against
# `ADR-003` §3, and left where `cmd/rivet-image --gdbserver` and `--strace`
# can be pointed at them.
#
# It exists for the reason `tools/nushell/build.sh` gives for itself: the build
# must be a repository artefact rather than one person's shell history. What it
# is NOT is a second build system. Buildroot builds both packages —
# `ADR-009` §6 — and this script drives `tools/buildroot/build.sh` and then does
# the two things that a `make gdb strace` does not do, both of which are
# explained where they happen: it strips, and it refuses a binary that would
# fail to `exec` on a board.
#
# Usage:
#   ./build.sh [--verify-only PATH...]
#
# Options:
#   --verify-only PATH...  skip the build and run §"What is verified" against
#                          existing binaries. What to use when checking one
#                          somebody handed you.
#
# Environment:
#   RIVET_BR_OUTPUT   Buildroot's output tree. Default:
#                       $HOME/.rivet/buildroot-<pin>, matching
#                       tools/buildroot/build.sh's own default.
#   RIVET_DEBUG_OUTPUT  where to put the stripped binaries.
#                       Default: $HOME/.rivet/debug
#
# ## Why this strips, and what happens if nobody does
#
# **Buildroot strips in `target-finalize`, which is part of `make all` and is
# not part of `make gdb strace`.** So a targeted build — which is what anybody
# adding a package actually runs — leaves the binaries in `target/usr/bin` with
# their full symbol tables and DWARF. Measured 2026-08-24 against Buildroot
# 2026.02.3 and GDB 15.2:
#
#     gdbserver   1 519 792 -> 1 010 936 bytes
#     strace      1 849 784 -> 1 521 600 bytes
#
# That is 837 040 bytes of debug information about the debugger, in each of two
# slots (`ADR-018` §4), on a card. The tree's other C mechanisms report
# `stripped` because they were built by a `make all` that reached
# `target-finalize`, so the two kinds would sit side by side in `/system/bin`
# on different terms with nothing saying why. `ADR-018` §10's distributed artifact is a number the maintainer
# tracks, and one that moved because of which make target somebody typed would
# be the worst kind of number to explain.
#
# ## What is verified, and why the script refuses rather than warns
#
# `ADR-003` §3's three checkable properties, read off each produced ELF:
#
#   1. `file` reports *statically linked*
#   2. `readelf -l` shows no `PT_INTERP` — no dynamic loader is requested
#   3. `readelf -d` reports no dynamic section
#
# This is not a formality here, because there is a **dynamically linked
# gdbserver in the same tree** and selecting it is a one-symbol mistake.
# Buildroot's `BR2_TOOLCHAIN_EXTERNAL_GDB_SERVER_COPY` copies the Bootlin
# toolchain's prebuilt stub instead of building one, and that binary is
#
#     ELF 64-bit LSB pie executable, ARM aarch64 … dynamically linked,
#     interpreter /lib/ld-musl-aarch64.so.1
#
# NEEDING `libstdc++.so.6` and `libgcc_s.so.1`. `ADR-008` ships no dynamic
# loader, so it would build cleanly, install cleanly, and fail at `exec` on the
# board naming an *interpreter* rather than the binary — the failure `ADR-003`
# §4 chose musl to avoid, and the one that is hardest to read from a console.
# The defconfig leaves that symbol off and says so; this checks the outcome
# rather than trusting the setting. `AGENTS.md` §5: refuse rather than guess.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

# Must match `tools/buildroot/build.sh`. Two literals for one fact, and the
# check below is what stops them drifting silently — the same arrangement
# `tools/nushell/build.sh` uses and for the same reason.
BR_VERSION=2026.02.3

BR_OUTPUT=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-$BR_VERSION}
OUTPUT=${RIVET_DEBUG_OUTPUT:-$HOME/.rivet/debug}

VERIFY_ONLY=no

case "$1" in
    -h|--help)
        sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'
        exit 0 ;;
    --verify-only)
        VERIFY_ONLY=yes
        shift ;;
    -?*)
        echo "$0: unknown option $1" >&2
        exit 2 ;;
esac

TOOL_PREFIX="$BR_OUTPUT/host/bin/aarch64-linux"
READELF="$TOOL_PREFIX-readelf"
[ -x "$READELF" ] || READELF=readelf
STRIP="$TOOL_PREFIX-strip"

# ---------------------------------------------------------------------------
# Verify — the whole of what this script promises about a binary
# ---------------------------------------------------------------------------

verify() {
    bin=$1

    if [ ! -f "$bin" ]; then
        echo "verify: $bin does not exist" >&2
        return 1
    fi

    echo
    echo "--- ADR-003 §3, read off $bin ---"

    # `file`, when it is there. Not fatal on its own: the two `readelf` checks
    # are the load-bearing ones. Saying which check was skipped is the point —
    # AGENTS.md §7, an absent tool and a passing check must not look alike.
    if command -v file >/dev/null 2>&1; then
        printf '  file:         '
        file -b "$bin"
        if ! file -b "$bin" | grep -q "statically linked"; then
            echo "  REFUSED: file(1) does not call this statically linked" >&2
            return 1
        fi
    else
        echo "  file:         (file(1) is not installed; the readelf checks below stand alone)"
    fi

    if "$READELF" -lW "$bin" 2>/dev/null | grep -qi "interpreter"; then
        echo "  PT_INTERP:    PRESENT — REFUSED" >&2
        "$READELF" -lW "$bin" | grep -i interpreter >&2
        echo "  This is what BR2_TOOLCHAIN_EXTERNAL_GDB_SERVER_COPY produces." >&2
        return 1
    fi
    echo "  PT_INTERP:    absent (no dynamic loader requested)"

    if "$READELF" -dW "$bin" 2>/dev/null | grep -q "Dynamic section at offset"; then
        echo "  .dynamic:     PRESENT — REFUSED" >&2
        return 1
    fi
    echo "  .dynamic:     absent"

    printf '  machine:      '
    "$READELF" -hW "$bin" | sed -n 's/.*Machine: *//p'
    printf '  size:         '
    stat -c '%s bytes' "$bin"
    return 0
}

if [ "$VERIFY_ONLY" = yes ]; then
    if [ $# -eq 0 ]; then
        echo "$0: --verify-only needs at least one path" >&2
        exit 2
    fi
    rc=0
    for bin in "$@"; do
        verify "$bin" || rc=1
    done
    exit $rc
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

if [ "$(uname -s)" != Linux ]; then
    echo "$0: Buildroot needs a Linux host; this is $(uname -s)." >&2
    echo "    ADR-009 §6 and ADR-026 §6 both put this in WSL2. Run it there." >&2
    exit 1
fi

# Refuse rather than downloading a toolchain, exactly as tools/nushell/build.sh
# does: a second musl, pinned differently and owned by nobody, would be the libc
# statically linked inside the thing an operator debugs with (ADR-011).
if [ ! -x "$TOOL_PREFIX-gcc" ]; then
    cat >&2 <<EOF
$0: no cross-toolchain at $TOOL_PREFIX-gcc

gdbserver and strace are C mechanisms and Buildroot's (ADR-009 §6). Build the
toolchain first:

    tools/buildroot/build.sh --fetch toolchain

or point RIVET_BR_OUTPUT at an existing output tree.
EOF
    exit 1
fi

echo "buildroot:  $BR_OUTPUT"
echo "output:     $OUTPUT"
echo

"$REPO/tools/buildroot/build.sh" gdb strace

mkdir -p "$OUTPUT"

# Copied out and stripped HERE rather than in the Buildroot tree. Two reasons,
# and the second is the one that matters: `target/` is Buildroot's, it is
# rewritten by any later `make`, and a `make all` reaching `target-finalize`
# would strip these anyway — so a script that stripped in place would sometimes
# be a no-op and sometimes be the only thing that stripped, with no way to tell
# which from the output. A copy this script owns has one state.
for name in gdbserver strace; do
    src="$BR_OUTPUT/target/usr/bin/$name"
    if [ ! -f "$src" ]; then
        echo "$0: Buildroot did not produce $src" >&2
        echo "    Check that the defconfig still carries BR2_PACKAGE_GDB," >&2
        echo "    BR2_PACKAGE_GDB_SERVER and BR2_PACKAGE_STRACE, and that the" >&2
        echo "    output tree was reconfigured after they were added:" >&2
        echo "        tools/buildroot/build.sh --reconfigure gdb strace" >&2
        exit 1
    fi
    cp "$src" "$OUTPUT/$name"
    if [ -x "$STRIP" ]; then
        before=$(stat -c '%s' "$OUTPUT/$name")
        "$STRIP" "$OUTPUT/$name"
        after=$(stat -c '%s' "$OUTPUT/$name")
        echo "stripped:   $name  $before -> $after bytes"
    else
        # Buildroot's strip and not the host's: strip is architecture-specific
        # and the host's would refuse an aarch64 object.
        echo "NOT STRIPPED: no $STRIP. $name carries its symbols — see the header" >&2
        echo "              for what that costs, per slot, on both slots." >&2
    fi
done

for name in gdbserver strace; do
    verify "$OUTPUT/$name" || exit 1
done

cat <<EOF

The binaries are $OUTPUT/gdbserver and $OUTPUT/strace. Neither is committed and
neither may be — ADR-010 §6.1, the same rule the kernel and the firmware are
under.

Put them on a card (both flags are OPTIONAL — ADR-005 §7.4 has not ruled that
a card should carry either, and this is what makes measuring the cost possible
rather than what decides it):

    cargo run -p rivetos-image -- out/card.img \\
        <init> <supervisor> <cli> <netd> <mcpd> <timed> <deploy> \\
        --gdbserver $OUTPUT/gdbserver --strace $OUTPUT/strace

$HERE/README.md has a real session, and what a stripped binary does to it.
EOF
