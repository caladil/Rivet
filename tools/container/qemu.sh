#!/bin/sh
# The QEMU loop, in the same box the card build runs in.
#
# `tools/container/build-card.sh` is the pattern and most of this file's
# reasoning is that one's — read its header for the Git Bash path rewriting, the
# `//wsl$/…` mount that silently lands EMPTY, and why the container paths are
# fixed constants. What is different here is the inputs.
#
# Usage:
#   tools/container/qemu.sh                     the gate: tools/qemu/smoke.sh
#   tools/container/qemu.sh <init> <supervisor> the narrower forms, passed through
#   tools/container/qemu.sh --run -- <args>     tools/qemu/run.sh instead
#   tools/container/qemu.sh --shell             a shell, to look around
#
# Runner options (everything else is passed to the script being run):
#   --run             run tools/qemu/run.sh, not smoke.sh. Everything after
#                     `--` is that script's argument vector.
#   --shell           drop into a shell in the container
#   --print-command   print the docker command and exit, running nothing
#   --rebuild-image   rebuild the container image even if it already exists
#   -h, --help        this text
#
# Environment:
#   RIVET_BR_VOLUME   a docker volume holding the Buildroot output, mounted
#                       READ-ONLY at /br. It must carry images/Image — see below.
#                       Default: rivet-br, if that volume exists.
#   RIVET_BR_OUTPUT   a host DIRECTORY holding the same, if the tree is
#                       bind-mountable. Default $HOME/.rivet/buildroot-*.
#   RIVET_DOCKER      the container command. Default `docker`; `podman` works.
#   RIVET_CACHE_TAG   suffix for the cache volume names. Default: empty.
#
# ## Why the kernel is the whole of the input question
#
# `tools/qemu/smoke.sh`'s control-plane stage needs a network, and a network
# under `-M virt` needs a **virtio-net driver in the kernel**. The Raspberry
# Pi's own `kernel8.img` has none — verified 2026-08-23 across virtio-mmio,
# virtio-pci and USB-ethernet — so `run.sh` refuses it by name rather than
# producing a console that looks like a working boot with a broken Rivet in it
# (`ADR-024` §3).
#
# The kernel that does work is `ADR-024`'s own, which `tools/buildroot/build.sh`
# builds and leaves at `images/Image`. **That is hours of work and is not built
# in the container**, for the Dockerfile's reason: it is a separate question.
# So it is mounted in, at `/br`, exactly as the card build's cross toolchain is
# — and `RIVET_KERNEL` points into that mount rather than at anybody's home
# directory.
#
# **Populate the volume once** (the same recipe `build-card.sh` documents for
# `host` and the two `target` binaries — this is the third thing it needs):
#
#   docker run --rm -v rivet-br:/br -v "<dir holding Image>:/in:ro" \
#       rivet-card-build:2 sh -c 'mkdir -p /br/images && cp /in/Image /br/images/'
#
# ## What it does NOT need
#
# **No port publishing.** The guest's control plane is reached through QEMU's
# own `hostfwd`, which lands on the CONTAINER's loopback, and the client —
# `cargo rivet run` — is built and run in the same container. Nothing crosses
# the container boundary, so there is no `-p` here and a second run cannot
# collide with the host's ports.
#
# **No privilege.** `-M virt` with TCG needs no `/dev/kvm`, which is what makes
# this run identically on a Windows host where there is no KVM to pass through.
# It is slower than a native boot and the timeouts in `smoke.sh` are set from
# the supervisor's own constants rather than from a stopwatch, so that is a
# speed cost and not a correctness one.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

DOCKER=${RIVET_DOCKER:-docker}
# The platform every image here is built and run as. The pinned Arch base and
# Buildroot's Bootlin toolchain exist only for x86-64, so an arm64 host (Apple
# Silicon) runs them emulated; ADR-062 §4. On an x86-64 host this is a no-op.
PLATFORM="--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}"

# The same image the card build uses, and deliberately the same number: one
# image, one place to bump it. `tools/container/build-card.sh` owns the
# constant's documentation.
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
C_BR=/br
# Read out of the mount rather than written into `smoke.sh`, for the reason the
# Dockerfile gives: a path in a tracked file that names somebody's home
# directory is the build that lives in one person's scrollback (`ADR-026` §6).
C_KERNEL=$C_BR/images/Image

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

die() { echo "tools/container/qemu.sh: $*" >&2; exit 1; }

# --- runner options ---------------------------------------------------------

REBUILD=no
MODE=smoke
PRINT_ONLY=no
PASSTHROUGH=""
while [ $# -gt 0 ]; do
    case "$1" in
        --rebuild-image) REBUILD=yes ;;
        --shell) MODE=shell ;;
        --run) MODE=run ;;
        --print-command) PRINT_ONLY=yes ;;
        --) shift; break ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) PASSTHROUGH="$PASSTHROUGH $1" ;;
    esac
    shift
done
# Everything after `--`, if there was one.
while [ $# -gt 0 ]; do
    PASSTHROUGH="$PASSTHROUGH $1"
    shift
done

# --- the Buildroot output, and the kernel inside it -------------------------

BR_VERSION=2026.02.3
BR_VOLUME=${RIVET_BR_VOLUME:-}
BR_OUTPUT=${RIVET_BR_OUTPUT:-}

if ! command -v "${DOCKER%% *}" >/dev/null 2>&1; then
    die "no '${DOCKER%% *}' on PATH. tools/container/build-card.sh's header says what to do."
fi

# **Defaulting to the volume rather than to a home directory**, which is the
# opposite of `build-card.sh` and is the newer of the two answers: on this
# project's Windows workstation the tree is inside a WSL distro that Windows
# Docker cannot bind-mount, and pointing `-v` at `//wsl$/…` mounts an EMPTY
# directory instead of failing. The volume is a Linux filesystem the daemon
# already owns, so it works on every host.
if [ -z "$BR_VOLUME" ] && [ -z "$BR_OUTPUT" ]; then
    if "$DOCKER" volume inspect rivet-br >/dev/null 2>&1; then
        BR_VOLUME=rivet-br
    else
        BR_OUTPUT=$HOME/.rivet/buildroot-$BR_VERSION
    fi
fi

if [ -n "$BR_VOLUME" ]; then
    "$DOCKER" volume inspect "$BR_VOLUME" >/dev/null 2>&1 \
        || die "RIVET_BR_VOLUME=$BR_VOLUME is not a docker volume. \`docker volume ls\` lists them."
    BR_MOUNT="-v $BR_VOLUME:$C_BR:ro"
    BR_SAYS="docker volume $BR_VOLUME"
else
    [ -d "$BR_OUTPUT" ] || die "no Buildroot output tree at $BR_OUTPUT.
    RIVET_BR_OUTPUT names one, or RIVET_BR_VOLUME names a docker volume holding
    it. If the tree is inside WSL, Windows Docker cannot mount it — a //wsl\$/…
    path mounts EMPTY rather than failing, which is why the volume exists."
    BR_MOUNT="-v $(hostpath "$BR_OUTPUT"):$C_BR:ro"
    BR_SAYS="$BR_OUTPUT"
fi

# --- the image --------------------------------------------------------------

have_image() { $DOCKER image inspect "$IMAGE" >/dev/null 2>&1; }

if [ "$REBUILD" = yes ] || ! have_image; then
    echo "--- building $IMAGE (this happens once) ---"
    $DOCKER build $PLATFORM -t "$IMAGE" \
        -f "$(hostpath "$HERE/Dockerfile")" \
        "$(hostpath "$HERE")"
fi

# **The kernel is checked BEFORE anything is built**, which is minutes and one
# whole workspace compile earlier than `run.sh` would refuse. It is checked
# inside the container because that is the only place the volume exists —
# `docker volume inspect` says a volume is there and says nothing about what is
# in it, and an empty `/br` is exactly the failure this is guarding against.
if [ "$MODE" != shell ] && [ "$PRINT_ONLY" != yes ]; then
    # shellcheck disable=SC2086
    if ! $DOCKER run $PLATFORM --rm $BR_MOUNT "$IMAGE" /bin/sh -c "[ -f $C_KERNEL ]"; then
        cat >&2 <<EOF
tools/container/qemu.sh: $BR_SAYS has no images/Image.

That is ADR-024's own kernel, and it is what --net needs: the Raspberry Pi's
kernel8.img contains no virtio-net driver at all, so run.sh refuses it by name
and the control-plane stage has no route into the guest.

tools/buildroot/build.sh builds one. To put it in the volume:

    docker run --rm -v $BR_VOLUME:/br -v "<dir holding Image>:/in:ro" \\
        $IMAGE sh -c 'mkdir -p /br/images && cp /in/Image /br/images/'

Refusing here rather than after the workspace has compiled.
EOF
        exit 1
    fi
fi

# --- mounts, environment, and who it runs as --------------------------------

CACHE_TAG=${RIVET_CACHE_TAG:-}
VOL_CARGO="rivet-card-cargo$CACHE_TAG"
VOL_TARGET="rivet-card-target$CACHE_TAG"

MOUNTS="-v $(hostpath "$REPO"):$C_REPO $BR_MOUNT"
MOUNTS="$MOUNTS -v $VOL_CARGO:/cargo -v $VOL_TARGET:/target"

# `RIVET_KERNEL` is the one variable `tools/qemu/run.sh` and `smoke.sh` read for
# this, and it points into the mount. A caller may still override it — a card
# build's own kernel8.img under the repository is reachable at $C_REPO — and
# that is left possible on purpose: the two-kernel trade is `ADR-026` §4.1's and
# is not this script's to remove.
#
# **`RIVET_BR_OUTPUT` is not only about the kernel.** `rivetos-mcpd` links
# `rivetos-tls`, which links `ring`, whose build compiles C — so the guest build
# needs the cross compiler at `host/bin/aarch64-linux-gcc`, exactly as
# `tools/card/build.sh` does and for exactly the reason `ADR-009` §3.7.4 prices.
# The same mount carries both.
ENVS="-e RIVET_KERNEL=${RIVET_KERNEL:-$C_KERNEL} -e RIVET_BR_OUTPUT=$C_BR"
ENVS="$ENVS -e HOME=/tmp -e TMPDIR=/tmp"

# `build-card.sh`'s argument, unchanged: on Linux a bind mount carries the
# host's uids through, so root-owned files would be left in the working tree.
#
# **And run.sh needs root or fakeroot** to `mknod /dev/console`, without which
# the kernel gives init no stdio and a working boot is indistinguishable from a
# hang (`ADR-026` §3). The image carries no fakeroot, so on Linux this is the
# one thing that has to be said out loud rather than discovered: pass
# RIVET_DOCKER_USER=0:0 if the run refuses there, and accept the ownership.
USER_ARG=""
if [ "$UNAME" = Linux ] && [ -z "${RIVET_DOCKER_USER-}" ]; then
    USER_ARG="--user $(id -u):$(id -g)"
elif [ -n "${RIVET_DOCKER_USER-}" ]; then
    USER_ARG="--user $RIVET_DOCKER_USER"
fi

case "$MODE" in
    shell) CMD="/bin/bash" ;;
    # shellcheck disable=SC2086
    run)   CMD="tools/qemu/run.sh$PASSTHROUGH" ;;
    # shellcheck disable=SC2086
    *)     CMD="tools/qemu/smoke.sh$PASSTHROUGH" ;;
esac

echo "--- $DOCKER run: $C_REPO from $REPO, $C_BR from $BR_SAYS ---"
echo "--- kernel: ${RIVET_KERNEL:-$C_KERNEL} ---"

if [ "$PRINT_ONLY" = yes ]; then
    # shellcheck disable=SC2086
    printf '%s run --rm %s -w %s %s %s %s /bin/sh -c %s\n' \
        "$DOCKER" "$USER_ARG" "$C_REPO" "$MOUNTS" "$ENVS" "$IMAGE" "'$CMD'"
    exit 0
fi

# `-i` and no `-t`, for `build-card.sh`'s reason plus one of this script's own:
# `run.sh` puts the guest console on stdio, and an allocated tty would merge the
# harness's stderr into it. `--shell` and `--run` are the interactive cases and
# a caller who wants a terminal for those adds `-t` through RIVET_DOCKER.
#
# shellcheck disable=SC2086
exec $DOCKER run $PLATFORM --rm -i $USER_ARG -w "$C_REPO" $MOUNTS $ENVS "$IMAGE" /bin/sh -c "$CMD"
