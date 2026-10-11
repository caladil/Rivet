#!/bin/sh
# Build a signed update payload in the container — the same command everywhere.
#
# This is a WRAPPER, exactly as `tools/container/build-card.sh` is one. It mounts
# what the build reads, translates the `RIVET_*` variables into container paths,
# and runs `tools/update/build.sh` unmodified. Everything that decides what a
# payload contains still lives there.
#
# Usage:
#   RIVET_SYSTEM_SEED=/path/to/seed \
#     tools/container/build-update.sh out/rivet-0.1.12-<commit>-…-card.img <commit>
#
# Runner options:
#   --print-command   print the docker command and exit, running nothing
#   -h, --help        this text
#
# Environment:
#   RIVET_SYSTEM_SEED  REQUIRED. The system signing seed on the HOST. Mounted
#                        READ-ONLY, for the length of one command. No default —
#                        `AGENTS.md` §8.3 forbids an absolute path in a tracked
#                        file, and that is unchanged by this script.
#   RIVET_DOCKER       the container command. Default `docker`; `podman` works.
#   RIVET_OUT          where payloads are written. Default <repo>/out.
#   RIVET_KEEP_PAYLOADS  retention, passed straight through to
#                        tools/update/build.sh (default 3 there).
#
# ## Why this exists — `ADR-026` §8
#
# Until 2026-08-30 this was the last thing that required a hand-configured WSL
# distro, and the reason given was that mounting a signing seed into a container
# was "a decision about a signing key rather than a plumbing detail".
#
# **It is a plumbing detail.** The seed is already read by a process on this
# host. A read-only bind mount gives it to a process on the SAME host, under the
# same user, for the length of one command. It is not baked into an image, not
# written into a layer, and nothing about it reaches a registry. `ADR-015` §4's
# rule is that the signing key never leaves the BUILD MACHINE — and a container
# on the build machine has not left it.
#
# The thing that would actually violate that rule is `COPY`ing a seed into an
# image. Nothing here does, and a change that started to would be the wrong turn.
#
# ## What is mounted, and nothing else
#
#   the repository   read-write, because the payload is written into out/
#   the seed         READ-ONLY, at a fixed container path
#
# No Buildroot tree: this reads a finished card image and the repository's own
# `cargo-rivet`, and needs no cross toolchain.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

DOCKER=${RIVET_DOCKER:-docker}
# The platform every image here is built and run as. The pinned Arch base and
# Buildroot's Bootlin toolchain exist only for x86-64, so an arm64 host (Apple
# Silicon) runs them emulated; ADR-062 §4. On an x86-64 host this is a no-op.
PLATFORM="--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}"

# The same image the card build uses; see tools/container/build-card.sh for why
# the number is explicit rather than a content hash.
# **The image tag, read from ONE file** - `tools/container/image-version`,
# and see the `.md` beside it for why. Four scripts spelled this number
# until 2026-09-02, which is four places a bump can miss one.
# `tr -d` because a checkout can hand this back with a CRLF whatever
# `.gitattributes` says - proven on 2026-09-02, when seven tracked scripts
# were found defying `*.sh text eol=lf`. A trailing \r here would make the
# tag `rivet-card-build:3\r`, which matches no image.
IMAGE_VERSION=$(tr -d '\r\n' <"$HERE/image-version")
IMAGE=rivet-card-build:$IMAGE_VERSION

C_REPO=/rivet
C_SEED=/seed/system.seed

# --- platform ---------------------------------------------------------------
#
# Identical to build-card.sh's, and deliberately duplicated rather than sourced:
# these two scripts are each meant to be readable on their own, and a shared
# fragment would make the mount handling a third file nobody opens. If a third
# wrapper appears, that is the moment to extract it.

UNAME=$(uname -s 2>/dev/null || echo unknown)
IS_MSYS=no
case "$UNAME" in
    MINGW*|MSYS*|CYGWIN*) IS_MSYS=yes ;;
esac

if [ "$IS_MSYS" = yes ]; then
    MSYS_NO_PATHCONV=1
    MSYS2_ARG_CONV_EXCL='*'
    export MSYS_NO_PATHCONV MSYS2_ARG_CONV_EXCL
fi

hostpath() {
    if [ "$IS_MSYS" = yes ]; then
        cygpath -m "$1"
    else
        printf '%s' "$1"
    fi
}

die() { echo "tools/container/build-update.sh: $*" >&2; exit 1; }

PRINT_ONLY=no
ARGS=""
while [ $# -gt 0 ]; do
    case "$1" in
        --print-command) PRINT_ONLY=yes ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) ARGS="$ARGS $1" ;;
    esac
    shift
done

# --- what the build reads, refused early --------------------------------------
#
# Each of these costs a container start and several seconds to discover inside,
# and the message from in there names a container path the caller never typed.

SEED=${RIVET_SYSTEM_SEED:-}
[ -n "$SEED" ] || die "RIVET_SYSTEM_SEED is not set, and there is deliberately no default.
    It names the system signing seed on THIS machine. AGENTS.md §8.3 keeps an
    absolute path out of a tracked file, so the variable is the interface.
    It is mounted read-only for the length of one command — ADR-026 §8.2."
[ -f "$SEED" ] || die "RIVET_SYSTEM_SEED=$SEED is not a file."

set -- $ARGS
[ $# -ge 2 ] || die "usage: tools/container/build-update.sh <card.img> <describe>
    <card.img>  a whole-device image from tools/card/build.sh, inside this repository
    <describe>  the version that image was built with, from the HOST's git"

IMG=$1
case "$IMG" in
    /*) die "name the image RELATIVE to the repository — it is mounted at $C_REPO,
    and an absolute host path does not exist inside the container." ;;
esac
[ -f "$REPO/$IMG" ] || die "$IMG is not a file under $REPO."

# --- the image ---------------------------------------------------------------

if ! "$DOCKER" image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "--- building $IMAGE (this happens once) ---"
    # Both paths through `hostpath`, for the reason at the top of
    # build-card.sh: with MSYS path conversion turned off, Docker Desktop does
    # not accept `/c/Users/...` and needs `C:/Users/...`. Getting this wrong
    # fails with "unable to prepare context: path not found", which reads as a
    # missing directory rather than as a path form.
    "$DOCKER" build $PLATFORM -t "$IMAGE" \
        -f "$(hostpath "$HERE/Dockerfile")" \
        "$(hostpath "$HERE")"
fi

OUT_DIR=${RIVET_OUT:-$REPO/out}
mkdir -p "$OUT_DIR"

MOUNTS="-v $(hostpath "$REPO"):$C_REPO"
MOUNTS="$MOUNTS -v $(hostpath "$OUT_DIR"):$C_REPO/out"
MOUNTS="$MOUNTS -v $(hostpath "$SEED"):$C_SEED:ro"

# HOME and TMPDIR are set because the container may run as any uid and nothing
# in it assumes a home directory exists — build-card.sh's reasoning, verbatim.
ENVS="-e RIVET_SYSTEM_SEED=$C_SEED -e HOME=/tmp -e TMPDIR=/tmp"
# Retention, passed straight through as build-card.sh passes RIVET_KEEP — a
# variable this wrapper does not name does not reach the container at all.
if [ -n "${RIVET_KEEP_PAYLOADS:-}" ]; then
    ENVS="$ENVS -e RIVET_KEEP_PAYLOADS=$RIVET_KEEP_PAYLOADS"
fi

CMD="$DOCKER run $PLATFORM --rm -w $C_REPO $MOUNTS -v rivet-card-cargo:/cargo -v rivet-card-target:/target $ENVS $IMAGE /bin/sh -c 'sh tools/update/build.sh $*'"

if [ "$PRINT_ONLY" = yes ]; then
    echo "$CMD"
    exit 0
fi

echo "--- $DOCKER run: $C_REPO from $REPO, the seed read-only from the host ---"
# shellcheck disable=SC2086
exec $DOCKER run $PLATFORM --rm -w "$C_REPO" $MOUNTS \
    -v rivet-card-cargo:/cargo -v rivet-card-target:/target \
    $ENVS "$IMAGE" /bin/sh -c "sh tools/update/build.sh $*"
