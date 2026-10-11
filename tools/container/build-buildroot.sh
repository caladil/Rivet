#!/bin/sh
# Build Buildroot in the container — same command on macOS, Windows and Linux.
#
# This is a WRAPPER, not a second build. It mounts the repository, provides the
# source tree and the output tree as Docker volumes, and runs
# `tools/buildroot/build.sh` — the same script, unmodified. Everything that
# decides what Buildroot produces still lives there and in
# `tools/buildroot/configs/rivet_aarch64_defconfig`.
#
# Usage:
#   tools/container/build-buildroot.sh --fetch          # first run: download Buildroot
#   tools/container/build-buildroot.sh --reconfigure     # after editing the defconfig
#   tools/container/build-buildroot.sh linux             # just the kernel
#
# Runner options (everything else is passed through to tools/buildroot/build.sh):
#   --rebuild-image   rebuild the container image even if it already exists
#   --shell           drop into a shell in the container instead of building
#   --print-command   print the docker command and exit, running nothing
#   -h, --help        this text
#
# Environment:
#   RIVET_DOCKER          the container command. Default `docker`; `podman` works.
#   RIVET_BR_VOLUME       the volume the OUTPUT is written to. Default
#                           `rivet-br`, which is what build-card.sh reads.
#   RIVET_SRC_VOLUME      the volume holding the Buildroot source tree and its
#                           download cache. Default `rivet-buildroot-src`.
#   JLEVEL                parallelism, passed through. Default: the container's nproc.
#
# ## Why the default output volume was `rivet-br-next` for a day
#
# ~~`rivet-br` is what `tools/container/build-card.sh` reads, and a card cannot
# be built without it. A Buildroot run that fails halfway leaves a partial tree;
# if that tree were `rivet-br`, a failed experiment would take the ability to
# build cards with it, and the way back is hours.~~
#
# **Adopted 2026-08-30 and the second volume is gone.** The caution was right
# while the tree was unproven: the old `rivet-br` held the only working
# toolchain, and losing it to a half-finished build would have cost hours. Six
# cards, a kernel swap and a boot later it is proven, so the new tree was
# copied into `rivet-br` and `rivet-br-next` deleted — one volume, the default
# everywhere, and no environment variable to remember.
#
# **If this caution is ever needed again, it is one word**: point
# `RIVET_BR_VOLUME` somewhere new, prove the tree, then promote it.
#
# ## Why the source tree is a volume rather than a host directory
#
# The same reason `build-card.sh` gives for the output tree, and on Windows it
# is correctness rather than speed: a Buildroot tree is full of symlinks,
# executable bits and case-sensitive filenames, and a Windows bind mount mangles
# all three quietly. The repository itself is still bind-mounted, because this
# build only READS it.
#
# ## It takes hours
#
# `ADR-026` §8 and `tools/container/Dockerfile`'s own note. A toolchain, a
# kernel and every C mechanism are built from source. `--fetch` additionally
# downloads Buildroot 2026.02.3 and verifies its sha256 before unpacking.
set -e

DOCKER=${RIVET_DOCKER:-docker}
# The platform every image here is built and run as. The pinned Arch base and
# Buildroot's Bootlin toolchain exist only for x86-64, so an arm64 host (Apple
# Silicon) runs them emulated; ADR-062 §4. On an x86-64 host this is a no-op.
PLATFORM="--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}"
IMAGE=rivet-buildroot:2
BR_VOLUME=${RIVET_BR_VOLUME:-rivet-br}
SRC_VOLUME=${RIVET_SRC_VOLUME:-rivet-buildroot-src}

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

die() { echo "$0: $*" >&2; exit 1; }

# Git Bash rewrites an argument that looks like a Unix path into a Windows one
# before the program sees it, which turns a container path into nonsense.
# `build-card.sh` carries the same function and the same explanation.
# **The Git Bash path problem, and only HALF of it was handled here.**
#
# Under MSYS the shell rewrites anything that looks like a Unix path in an
# argument before the program sees it -- and it rewrites the CONTAINER side too,
# which is the half that surprises people. `tools/container/build-card.sh`
# documents this at length and does both halves; this script had `hostpath` for
# the host side and nothing for the other, so `-w /rivet` reached Docker as
# `-w 'C:/Program Files/Git/rivet'` and every run refused with
#
#     the working directory 'C:/Program Files/Git/rivet' is invalid
#
# **It had therefore never worked from Git Bash at all**, which went unnoticed
# because a Buildroot build is something somebody does once and it was done
# from inside WSL, where this takes the branch that needs no conversion.
# Found 2026-08-31 by running it (`AGENTS.md` §6).
case "$(uname -s 2>/dev/null || echo unknown)" in
    MINGW*|MSYS*|CYGWIN*)
        MSYS_NO_PATHCONV=1
        MSYS2_ARG_CONV_EXCL='*'
        export MSYS_NO_PATHCONV MSYS2_ARG_CONV_EXCL
        ;;
esac

hostpath() {
    case "$(uname -s)" in
        MINGW*|MSYS*) cygpath -m "$1" ;;
        *) printf '%s' "$1" ;;
    esac
}

MODE=build
REBUILD_IMAGE=no
PASSTHROUGH=""
for arg in "$@"; do
    case "$arg" in
        --rebuild-image) REBUILD_IMAGE=yes ;;
        --shell) MODE=shell ;;
        --print-command) MODE=print ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//;$d'; exit 0 ;;
        *) PASSTHROUGH="$PASSTHROUGH $arg" ;;
    esac
done

command -v "${DOCKER%% *}" >/dev/null 2>&1 \
    || die "\`$DOCKER\` is not on PATH. Docker Desktop puts docker.exe on the PATH Git Bash sees."

# The volumes are created explicitly rather than left to `docker run`, so that
# an empty one is a state this script chose rather than a typo it inherited: a
# mistyped volume name would otherwise be silently created and the build would
# start from nothing, an hour before anyone noticed.
for v in "$SRC_VOLUME" "$BR_VOLUME"; do
    "$DOCKER" volume inspect "$v" >/dev/null 2>&1 || "$DOCKER" volume create "$v" >/dev/null
done

if [ "$REBUILD_IMAGE" = yes ] || ! "$DOCKER" image inspect "$IMAGE" >/dev/null 2>&1; then
    [ "$MODE" = print ] || "$DOCKER" build $PLATFORM -f "$(hostpath "$HERE/Dockerfile.buildroot")" -t "$IMAGE" "$(hostpath "$HERE")"
fi

# `$HOME` is set because `tools/buildroot/build.sh` composes its defaults from
# it, and an unset HOME would put a multi-gigabyte tree in `/`.
RUN_ARGS="--rm -w /rivet \
    -v $(hostpath "$REPO"):/rivet \
    -v $SRC_VOLUME:/src \
    -v $BR_VOLUME:/br \
    -e HOME=/home/builder \
    -e RIVET_BUILDROOT=/src/buildroot-2026.02.3 \
    -e RIVET_BR_OUTPUT=/br"
[ -n "${JLEVEL:-}" ] && RUN_ARGS="$RUN_ARGS -e JLEVEL=$JLEVEL"

if [ "$MODE" = shell ]; then
    COMMAND="/bin/bash"
else
    COMMAND="tools/buildroot/build.sh$PASSTHROUGH"
fi

if [ "$MODE" = print ]; then
    echo "$DOCKER run $PLATFORM $RUN_ARGS $IMAGE /bin/sh -c '$COMMAND'"
    exit 0
fi

echo "--- buildroot in $IMAGE: source $SRC_VOLUME, output $BR_VOLUME ---"
echo "--- this takes hours; ADR-026 §8 ---"
TTY=""
[ "$MODE" = shell ] && TTY="-it"
# **`exec` rather than a call, and the exit status is the container's.**
#
# Worth stating because the failure above was reported as a SUCCESS by a caller
# that only looked at the exit code: `docker run` refused the mangled `-w`,
# printed its error, and this script had already `exec`ed so there was nothing
# left to notice. `exec` is right -- there is nothing to do afterwards -- and the
# lesson is the caller's: a build that printed an error and exited 0 was Docker
# failing before the container started, which is a different thing from a build
# that ran and failed.
# shellcheck disable=SC2086
exec "$DOCKER" run $PLATFORM $RUN_ARGS $TTY "$IMAGE" /bin/sh -c "$COMMAND"
