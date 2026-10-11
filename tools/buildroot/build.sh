#!/bin/sh
# Drive Buildroot from Rivet's own tree.
#
# `ADR-009` §6 ruled Buildroot in for **the musl cross-toolchain and the C
# mechanisms**, and ruled image assembly out — that stays in Rivet's own Rust
# tooling. `ADR-024` §5 added the kernel, built from Rivet's fragment. This
# script is the whole of the interface between the two, and it exists so the
# build is a repository artefact rather than one person's shell history — the
# same argument `ADR-026` §6 made for `tools/qemu/`.
#
# Usage:
#   ./build.sh [options] [buildroot targets...]
#
# Options:
#   --fetch          download and verify the pinned Buildroot release if the
#                    tree named by RIVET_BUILDROOT (or the default) is absent.
#                    Without this the script refuses rather than downloading
#                    130 MB of somebody else's source on a whim.
#   --reconfigure    regenerate BUILDROOT's .config from the defconfig,
#                    discarding any local `make menuconfig` edits. Needed after
#                    editing the defconfig, because Buildroot will not notice on
#                    its own and will happily rebuild the previous configuration
#                    while you read a diff that says otherwise.
#
#                    **IT DOES NOT REGENERATE THE KERNEL'S .config, and that is
#                    a different file.** Added to this text 2026-09-04, after it
#                    cost a Bluetooth study two whole runs: the kernel's config
#                    is merged during the `linux` package's own configure step,
#                    which Buildroot re-runs on the fragments' MTIME. A fragment
#                    edited into place, or restored from a copy with an older
#                    timestamp, leaves the previous kernel config sitting there
#                    — and the build exits 0 in six seconds having rebuilt
#                    nothing, which reads exactly like a fast incremental build.
#
#                    `make -C $BR_TREE O=$BR_OUTPUT linux-reconfigure` is what
#                    forces it, and the same applies to every package:
#                    `<pkg>-reconfigure`. The Bluetooth study hit it twice, on
#                    the kernel and again on `bluez5_utils`.
#
#                    **`--show-config` is how you find out which happened**, and
#                    it is why that flag reads the produced .config back instead
#                    of trusting the merge.
#   --show-config    after BUILDING, print the kernel .config lines this
#                    project asked for, so a human can see what the merge
#                    actually produced. See "verify, do not assume" below.
#                    It ran before the build until 2026-08-31, which made it
#                    report the PREVIOUS kernel — see the block itself.
#
# Default target is `all`. `linux` alone builds just the kernel, which is what
# the QEMU loop needs and is much faster than the full set.
#
# Environment:
#   RIVET_BUILDROOT   the Buildroot source tree. Default: $HOME/buildroot-<pin>
#   RIVET_BR_OUTPUT   the build output directory. Default: $HOME/.rivet/buildroot-<pin>
#   JLEVEL              parallelism. Default: nproc.
#
# ## Why both defaults are outside the repository
#
# Two independent reasons, and each on its own is sufficient.
#
# 1. **Size.** A Buildroot output tree with a toolchain and a kernel is several
#    gigabytes. `.gitignore` would have to grow an entry, and an entry is a
#    thing that can be forgotten — keeping the output somewhere git cannot see
#    is the stronger gate (`AGENTS.md` §3's "the friction is the feature",
#    applied to build output).
# 2. **The development host is Windows** (`AGENTS.md` §10.5) and the repository
#    is reached from WSL over 9p. A kernel build on `/mnt/c` is slow enough to
#    change how often anyone runs it, and a loop nobody runs is `AGENTS.md` §4's
#    "test suite that cannot be run will rot silently".
#
# ## Verify, do not assume
#
# `merge_config.sh` is not authoritative about the result. A fragment can ask
# for a symbol whose dependencies are unmet and the merge will *warn* and carry
# on, leaving a kernel that is missing the thing the fragment was written to
# guarantee. `--show-config` reads the produced `.config` back and shows what is
# actually in it. `AGENTS.md` §6: what you verified, and by what method.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

# Pinned, not "latest". A Buildroot release changes package versions, toolchain
# versions and defconfigs; an unpinned build means two people never build the
# same thing and a boot log records a kernel nobody can reproduce.
#
# 2026.02.x is a long-term-support series, which is what a project that wants a
# reproducible toolchain rather than the newest packages should be on.
BR_VERSION=2026.02.3
BR_SHA256=5a59e7501b0b4ec52c41f4bfa79412320e0b37eae5f719605a258e8d0c6fc7fb

BR_TREE=${RIVET_BUILDROOT:-$HOME/buildroot-$BR_VERSION}
BR_OUTPUT=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-$BR_VERSION}

DO_FETCH=no
DO_RECONFIGURE=no
DO_SHOW=no

usage() {
    cat >&2 <<'EOF'
usage: build.sh [--fetch] [--reconfigure] [--show-config] [buildroot targets...]

  --fetch          download and sha256-verify Buildroot if the tree is absent
  --reconfigure    regenerate .config from tools/buildroot/configs/, discarding
                   local menuconfig edits. Required after editing the defconfig
                   or a kernel fragment.
  --show-config    print what the kernel .config actually ended up with for the
                   symbols the fragments asked for

Targets are Buildroot's own. `linux` builds only the kernel; the default `all`
builds the toolchain, the kernel and the C mechanisms.

Environment: RIVET_BUILDROOT, RIVET_BR_OUTPUT, JLEVEL.
EOF
    exit 2
}

while [ $# -gt 0 ]; do
    case $1 in
        --fetch)        DO_FETCH=yes; shift ;;
        --reconfigure)  DO_RECONFIGURE=yes; shift ;;
        --show-config)  DO_SHOW=yes; shift ;;
        -h|--help)      usage ;;
        -*)             echo "$0: unknown option $1" >&2; usage ;;
        *)              break ;;
    esac
done

# Refuse rather than guess (`AGENTS.md` §3). Buildroot is a Linux build system;
# on anything else it fails deep inside a package with a message about a
# compiler, and the actual cause — wrong operating system — is nowhere in it.
if [ "$(uname -s)" != Linux ]; then
    echo "$0: Buildroot needs a Linux host; this is $(uname -s)." >&2
    echo "    ADR-009 §6 and ADR-026 §6 both put this in WSL2. Run it there." >&2
    exit 1
fi

# WSL2 appends the Windows PATH to the Linux one by default, which puts entries
# like `/mnt/c/Program Files/...` in it. Buildroot checks for that and stops
# with:
#
#     Your PATH contains spaces, TABs, and/or newline (\n) characters.
#     This doesn't work. Fix your PATH environment variable.
#
# The advice is not wrong, but this is the maintainer's interactive environment
# and `AGENTS.md` §1 is explicit that adapting to it is this script's problem
# and not his. So the offending entries are dropped for the life of this
# process only. Nothing Buildroot needs lives behind a path with a space in it —
# every one of them is a Windows interop entry — and dropping them silently
# would be its own defect, so they are named.
CLEAN_PATH=
DROPPED=
OLDIFS=$IFS
IFS=:
for dir in $PATH; do
    case $dir in
        *[[:space:]]*) DROPPED="$DROPPED $dir" ;;
        "")            ;;
        *)             CLEAN_PATH=${CLEAN_PATH:+$CLEAN_PATH:}$dir ;;
    esac
done
IFS=$OLDIFS
if [ -n "$DROPPED" ]; then
    echo "note: dropping $(set -- $DROPPED; echo $#) PATH entries containing whitespace"
    echo "      (Buildroot refuses such a PATH outright; these are WSL's Windows"
    echo "      interop entries and nothing in this build lives behind them)"
    PATH=$CLEAN_PATH
    export PATH
fi

# Buildroot refuses to run as root itself, but it refuses *late*. Saying so here
# costs a second instead of a download.
if [ "$(id -u)" = 0 ]; then
    echo "$0: refusing to run as root — Buildroot does too, and packages that" >&2
    echo "    slip past it can write outside the output tree. Run as your user." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# The Buildroot tree
# ---------------------------------------------------------------------------

if [ ! -d "$BR_TREE" ]; then
    if [ "$DO_FETCH" != yes ]; then
        echo "$0: no Buildroot tree at $BR_TREE" >&2
        echo "    Re-run with --fetch to download and verify buildroot-$BR_VERSION," >&2
        echo "    or point RIVET_BUILDROOT at an existing tree of that version." >&2
        exit 1
    fi

    TARBALL=$BR_TREE.tar.xz
    URL=https://buildroot.org/downloads/buildroot-$BR_VERSION.tar.xz
    echo "fetching $URL"
    if ! curl -fSL -o "$TARBALL" "$URL"; then
        echo "$0: download failed" >&2
        rm -f "$TARBALL"
        exit 1
    fi

    # Pinned by hash, like tools/fetch-firmware.sh. A download that does not
    # match is refused rather than used: the toolchain and the kernel that come
    # out of this tree are what runs as root on the target.
    got=$(sha256sum "$TARBALL" | cut -d' ' -f1)
    if [ "$got" != "$BR_SHA256" ]; then
        echo "$0: sha256 mismatch for buildroot-$BR_VERSION" >&2
        echo "    expected $BR_SHA256" >&2
        echo "    got      $got" >&2
        echo "    Refusing it. Either the pin is wrong or the download is not" >&2
        echo "    what it claims to be." >&2
        rm -f "$TARBALL"
        exit 1
    fi
    echo "sha256 verified"

    # Extract beside the destination and rename, so an interrupted extraction
    # cannot leave a half-tree that the next run mistakes for a good one.
    STAGE=$BR_TREE.unpacking.$$
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    tar -xf "$TARBALL" -C "$STAGE"
    mv "$STAGE/buildroot-$BR_VERSION" "$BR_TREE"
    rmdir "$STAGE"
    rm -f "$TARBALL"
fi

# The tree might predate this pin. Buildroot puts its version in its Makefile;
# checking it is cheap and the alternative is a build that silently uses a
# different package set from the one the record claims.
FOUND=$(sed -n 's/^export BR2_VERSION := *//p' "$BR_TREE/Makefile" | head -1)
if [ "$FOUND" != "$BR_VERSION" ]; then
    echo "$0: $BR_TREE is Buildroot '$FOUND', not the pinned $BR_VERSION." >&2
    echo "    Refusing rather than building against a version this repository" >&2
    echo "    has not recorded. Move it aside, or update the pin here on" >&2
    echo "    purpose and say so in ADR-024." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Configure
# ---------------------------------------------------------------------------

mkdir -p "$BR_OUTPUT"
BR_MAKE="make -C $BR_TREE O=$BR_OUTPUT BR2_EXTERNAL=$HERE"

if [ ! -f "$BR_OUTPUT/.config" ] || [ "$DO_RECONFIGURE" = yes ]; then
    echo "configuring from tools/buildroot/configs/rivet_aarch64_defconfig"
    $BR_MAKE rivet_aarch64_defconfig
elif [ "$HERE/configs/rivet_aarch64_defconfig" -nt "$BR_OUTPUT/.config" ] \
  || [ "$HERE/linux/rivet.config" -nt "$BR_OUTPUT/.config" ] \
  || [ "$HERE/linux/qemu-virt.config" -nt "$BR_OUTPUT/.config" ]; then
    # Say it rather than doing it. Silently reconfiguring would throw away a
    # `make menuconfig` session; silently *not* saying anything would build the
    # old configuration while the author reads the new file. Neither is
    # acceptable, so the build stops and the human chooses.
    echo "$0: the checked-in configuration is newer than $BR_OUTPUT/.config." >&2
    echo "    Buildroot will not notice, and would rebuild the previous" >&2
    echo "    configuration while you read the new one. Re-run with" >&2
    echo "    --reconfigure (this discards local menuconfig edits)." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

if [ $# -eq 0 ]; then
    set -- all
fi

JLEVEL=${JLEVEL:-$(nproc)}
echo
echo "buildroot:  $BR_TREE ($BR_VERSION)"
echo "output:     $BR_OUTPUT"
echo "targets:    $*"
echo "parallel:   $JLEVEL"
echo

$BR_MAKE JLEVEL="$JLEVEL" "$@"

# **AFTER the build, and it was BEFORE it until 2026-08-31.**
#
# Buildroot writes `build/linux-custom/.config` during the linux package's own
# configure step, which happens inside `make linux` — not during the defconfig
# step this script runs earlier. So a `--show-config` before the build read the
# .config of the PREVIOUS build and presented it as the result of this one.
#
# **That is the exact failure this flag exists to prevent**, committed by the
# flag. It cost a build on 2026-08-31: `CONFIG_DRM_VC4=y` was asked for, Kconfig
# silently downgraded it to `m` because `depends on SND && SND_SOC` were both
# modules, and the report printed beforehand could not have shown it. The
# second run found it by reading the .config directly.
#
# `AGENTS.md` §7 both ways, one level out: a guard that reports a stale reading
# is worse than no guard, because it is believed.
if [ "$DO_SHOW" = yes ]; then
    KCONF=$BR_OUTPUT/build/linux-custom/.config
    if [ ! -f "$KCONF" ]; then
        echo >&2
        echo "--show-config: no kernel .config yet at $KCONF." >&2
        echo "    It appears once 'linux' has been configured; run a build first." >&2
    else
        echo
        echo "--- what the fragments asked for, and what the kernel .config says ---"
        echo "    (a symbol reported 'absent' was requested and did NOT survive the"
        echo "     merge — its dependencies were unmet. That is a real failure, not"
        echo "     noise: AGENTS.md §7 on claims of absence cuts both ways.)"
        sed -n 's/^\(CONFIG_[A-Z0-9_]*\)=.*/\1/p;s/^# \(CONFIG_[A-Z0-9_]*\) is not set/\1/p' \
            "$HERE/linux/rivet.config" "$HERE/linux/qemu-virt.config" \
            | sort -u \
            | while read -r sym; do
                line=$(grep -E "^($sym=|# $sym is not set)" "$KCONF" || true)
                printf '  %-40s %s\n' "$sym" "${line:-ABSENT from .config}"
            done
    fi
fi


echo
echo "--- artefacts in $BR_OUTPUT/images ---"
ls -la "$BR_OUTPUT/images" 2>/dev/null || echo "  (none — that target produced no image)"
echo
echo "The kernel is images/Image. It is NOT committed and must not be:"
echo "ADR-026 §6 — the kernel is an input to the loop, not an artefact of it."
echo "Point the runner at it:  RIVET_KERNEL=$BR_OUTPUT/images/Image tools/qemu/run.sh ..."
