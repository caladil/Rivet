#!/bin/sh
# Build a card image in the container — same command on macOS, Windows and Linux.
#
# This is a WRAPPER, not a second build. It mounts the repository, the Buildroot
# output tree and whatever else the build was pointed at, translates the
# `RIVET_*` variables into paths that exist inside the container, and then runs
# `tools/card/build.sh` — the same script, unmodified. Everything that decides
# what goes on a card still lives there.
#
# Usage:
#   RIVET_DESCRIBE=$(tools/card/bump.sh | tail -1) tools/container/build-card.sh
#   tools/container/build-card.sh --device-mib 4096 --default-slot b
#
# Runner options (everything else is passed through to tools/card/build.sh):
#   --rebuild-image   rebuild the container image even if it already exists
#   --shell           drop into a shell in the container instead of building
#   --print-command   print the docker command and exit, running nothing
#   -h, --help        this text
#
# Environment — the same names tools/card/build.sh reads, taken from the HOST
# and rewritten to container paths:
#   RIVET_DESCRIBE    REQUIRED. The commit string for the image's name. Comes
#                       from the host's git, never the container's — ADR-027 §10
#                       and AGENTS.md §10.5. `tools/card/bump.sh` mints it.
#   RIVET_BR_OUTPUT   Buildroot's output tree. Default $HOME/.rivet/buildroot-*.
#                       Mounted READ-ONLY at /br. NOT built here: it is hours of
#                       work and a separate question (see the Dockerfile).
#   RIVET_NU          the nu binary. Mounted read-only. Optional.
#   RIVET_DOCUMENT    the first-boot provisioning document. Mounted read-only.
#   RIVET_STAGING     the staging bundle beside it — ADR-049. Mounted read-only.
#   RIVET_APPLICATIONS  space-separated signed .rvt payloads this card carries —
#                     ADR-050 §4. Each mounted read-only. A build asked to carry
#                     payloads with no application key REFUSES.
#   RIVET_IDENTITY    the device TLS identity directory. Mounted read-only.
#   RIVET_FLEET_REPOSITORY  a fleet repository directory `cargo rivet fleet
#                     publish` has filed objects under -- ADR-050 §5.1. Mounted
#                     READ-ONLY, like RIVET_IDENTITY: it is a host directory
#                     tools/card/build.sh only ever reads from, to resolve a
#                     role's required payloads by digest. Needs RIVET_DOCUMENT
#                     too, on tools/card/build.sh's own terms.
#   RIVET_KEEP        retention, passed straight through.
#   RIVET_NO_TRUST    passed straight through.
#
# Runner-only environment:
#   RIVET_DOCKER      the container command. Default `docker`; `podman` works.
#   RIVET_OUT         where the image is written. Default <repo>/out. Mounted
#                       over /rivet/out, so a build can put a 2 GiB file
#                       somewhere other than the repository.
#   RIVET_CACHE_TAG   suffix for the cache volume names, if you want two
#                       independent caches. Default: empty.
#
# ## Why this exists
#
# `sgdisk`, `mkfs.vfat`, `mcopy`, `mkfs.erofs`, `mkfs.ext4` and `debugfs` are
# the whole external dependency of a card build, and on the Windows workstation
# this project is developed on they exist only inside a hand-configured WSL
# distro. `AGENTS.md` §10 carries three separate hazards that arrangement
# created — CRLF reaching a shell, `git describe` disagreeing across the two
# sides, and Git Bash rewriting paths before WSL sees them. The container is one
# artifact that replaces the first of those and is honest about the other two.
#
# ## It asks for no privilege
#
# No `--privileged`, no `--cap-add`, no `--device`, no loop device. See the
# Dockerfile: `cmd/rivet-image` was written never to need root and has a test
# saying so. If a change here starts needing one, the wrong turn was earlier.
#
# ## The Git Bash path problem, and what is done about it
#
# Under Git Bash (MSYS) the shell rewrites anything that looks like a Unix path
# in an argument before the program sees it. `-v /some/host/dir:/rivet` becomes
# `-v C:/some/host/dir:C:/Program Files/Git/rivet` — the CONTAINER side is
# mangled too, which is the half that surprises people, and the mount silently
# lands somewhere useless.
#
# Two things are done, and both are needed:
#
#   1. `MSYS_NO_PATHCONV=1` and `MSYS2_ARG_CONV_EXCL='*'` turn the rewriting
#      off, so `/rivet` stays `/rivet`.
#   2. With it off, host paths are no longer converted either — and Docker
#      Desktop does not accept an MSYS path like `/c/Users/...`. So every host
#      path is put through `cygpath -m` explicitly, which yields `C:/Users/...`.
#
# Doing only (1) breaks the host side; doing only (2) breaks the container side.
#
# ## Where `docker` has to be, and the honest answer for Windows
#
# This script runs whatever `docker` is on PATH. On macOS and Linux that is the
# native one and there is nothing more to say.
#
# **On Windows there are two arrangements and only one of them removes WSL.**
# With Docker Desktop, `docker.exe` is on the Windows PATH, it accepts the
# `C:/...` mounts `hostpath` produces, and Git Bash is all that is needed — the
# WSL distro this project has been building in stops being required. If instead
# Docker lives *inside* a WSL distro, as it did on this workstation while this
# script was written, then `docker` is not on the Windows PATH at all and the
# script has to be run from inside that distro, where it takes the Linux branch
# and works unchanged. That is a perfectly good arrangement and it is **not**
# WSL-free: it replaces a hand-configured distro with a pinned container, which
# is the reproducibility half of the problem, and leaves the WSL hop in place.
#
# The WSL hop is deliberately NOT baked in here. A script that shelled out to
# `wsl.exe` would work on exactly one machine, which is the opposite of the
# point.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

DOCKER=${RIVET_DOCKER:-docker}
# The platform every image here is built and run as. The pinned Arch base and
# Buildroot's Bootlin toolchain exist only for x86-64, so an arm64 host (Apple
# Silicon) runs them emulated; ADR-062 §4. On an x86-64 host this is a no-op.
PLATFORM="--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}"

# Bump when the Dockerfile changes, so an existing image is not reused for a
# recipe it does not implement. A content hash would do this automatically and
# would need a hash utility whose name differs on macOS; an explicit number is
# one line and never lies about which platform it is on.
# **2 since 2026-08-30**, when `qemu-system-aarch64` and `cpio` joined the image
# so that `tools/qemu/` runs in here too (`tools/container/qemu.sh`).
# **The image tag, read from ONE file** - `tools/container/image-version`,
# and see the `.md` beside it for why. Four scripts spelled this number
# until 2026-09-02, which is four places a bump can miss one.
# `tr -d` because a checkout can hand this back with a CRLF whatever
# `.gitattributes` says - proven on 2026-09-02, when seven tracked scripts
# were found defying `*.sh text eol=lf`. A trailing \r here would make the
# tag `rivet-card-build:3\r`, which matches no image.
IMAGE_VERSION=$(tr -d '\r\n' <"$HERE/image-version")
IMAGE=rivet-card-build:$IMAGE_VERSION

# Container-side paths. Fixed constants, and that is deliberate: they are part
# of what `rustc` embeds in a binary, so holding them still is what lets a
# build on one operating system produce the same bytes as a build on another.
C_REPO=/rivet
C_BR=/br
C_OUT=/rivet/out
C_NU=/nu/nu
C_DOC=/provisioning/document.toml
# The staging bundle -- ADR-049. Beside the document, because they are two
# halves of one card and a person reading this list should see that.
C_STAGING=/provisioning/staged.txt
C_IDENTITY=/identity
# The fleet repository -- a host directory, mounted read-only on RIVET_IDENTITY's
# own footing: this build only ever reads objects out of it.
C_FLEET_REPO=/fleet-repository

# --- platform ---------------------------------------------------------------

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

# A host path in the form the container runtime will accept. Under MSYS that is
# a Windows path with forward slashes (`cygpath -m`); everywhere else the path
# is already what it should be.
hostpath() {
    if [ "$IS_MSYS" = yes ]; then
        cygpath -m "$1"
    else
        printf '%s' "$1"
    fi
}

# `$1`, as a POSIX path comparable against `$REPO`.
#
# **Fixed 2026-09-13 — the defect the provisioning dry run found.** `$REPO` is
# always POSIX (`cd ... && pwd`), but `RIVET_TRUSTED_KEY`/`RIVET_APP_KEY` come
# from a caller that may spell the identical file as a Windows path —
# `C:/src/Rivet/keys/my-system.pub` or
# `C:\src\Rivet\keys\my-system.pub` — because that is what a Windows
# tool prints and what a person pastes. Comparing that string against `$REPO`
# byte for byte never matches, so `under_repo` refused a key that WAS under
# the repository, and every caller of this script on Windows had to know to
# retype the path in POSIX form to be believed. `cygpath -u` is the same
# conversion `hostpath` already performs in the other direction; this is that
# fix, pointed at reading instead of at composing a mount.
normalize() {
    if [ "$IS_MSYS" = yes ]; then
        _p=$(cygpath -u "$1" 2>/dev/null || printf '%s' "$1")
    else
        _p=$1
    fi
    # A relative path is relative to where the command was run, which is how
    # a person types `keys/my-system.pub`; compared as-is, it never matched
    # `$REPO` and a key inside the repository was refused as outside it.
    case "$_p" in
        /*) printf '%s' "$_p" ;;
        *) printf '%s/%s' "$(pwd)" "$_p" ;;
    esac
}

# Is $1 inside $REPO? Used to decide whether a path the caller gave can be
# reached through the repository mount or needs one of its own.
under_repo() {
    case "$(normalize "$1")" in
        "$REPO"/*|"$REPO") return 0 ;;
        *) return 1 ;;
    esac
}

# The same path, as the container sees it through the repository mount.
repo_relative() {
    _n=$(normalize "$1")
    printf '%s%s' "$C_REPO" "${_n#"$REPO"}"
}

die() { echo "tools/container/build-card.sh: $*" >&2; exit 1; }

# --- runner options ---------------------------------------------------------

REBUILD=no
MODE=build
PRINT_ONLY=no
PASSTHROUGH=""
while [ $# -gt 0 ]; do
    case "$1" in
        --rebuild-image) REBUILD=yes ;;
        --shell) MODE=shell ;;
        --print-command) PRINT_ONLY=yes ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) PASSTHROUGH="$PASSTHROUGH $1" ;;
    esac
    shift
done

# --- the describe string, which must come from the host ---------------------
#
# Checked here rather than left to tools/card/build.sh, which would also refuse
# it — but only after an image build and a container start, and the operator
# would read the failure as a container problem. ADR-027 §10.2 rules the
# sequence; this is the first step of it, enforced early.
if [ -z "${RIVET_DESCRIBE:-}" ]; then
    cat >&2 <<'EOF'
tools/container/build-card.sh: RIVET_DESCRIBE is not set.

It is taken from the HOST's git and never computed inside the container. Under
Windows that matters twice over: WSL's git reads this CRLF working copy as
dirty (AGENTS.md §10.5), and so would a container's, so an image built from
either would claim `-dirty` for ever.

    RIVET_DESCRIBE=$(tools/card/bump.sh | tail -1) tools/container/build-card.sh
EOF
    exit 2
fi
case "$RIVET_DESCRIBE" in
    *-dirty) die "RIVET_DESCRIBE is '$RIVET_DESCRIBE' — ADR-027 §10.2 refuses a dirty tree." ;;
esac

# --- what gets mounted ------------------------------------------------------

BR_VERSION=2026.02.3

# **RIVET_BR_VOLUME — a Docker volume instead of a host directory.**
#
# Added 2026-08-29, after the Windows case turned out not to work at all.
#
# On this project's Windows workstation the Buildroot tree lives in the WSL
# `archlinux` distro's home, because that is where it was built. Windows Docker
# CANNOT bind-mount it. Pointing `-v` at `//wsl$/archlinux/home/…` does not
# fail: **Docker creates an empty directory and mounts that**, so the build gets
# a `/br` containing nothing and fails later, describing a missing compiler.
# That is a silent wrong answer, which is the failure mode this project spends
# the most effort removing — so it is named here rather than left to be
# rediscovered.
#
# The three real options were: copy 4.5 GB onto the Windows side; turn on Docker
# Desktop's WSL integration for the distro, which is a machine setting and not
# this script's business; or **put the tree in a Docker volume**, which is a
# Linux filesystem the daemon already owns. The volume wins on two counts beyond
# convenience: it preserves the symlinks the toolchain is made of
# (`host/bin/aarch64-linux-gcc` is one), and it needs only the ~477 MB the build
# actually reads rather than the whole tree.
#
# Populate one once:
#
#   wsl.exe -d archlinux -- tar -C ~/.rivet/buildroot-2026.02.3 \
#       -cf /mnt/c/Users/you/brtar/br.tar host target/usr/bin/git \
#       target/usr/libexec/git-core/git-http-backend
#   docker run --rm -v rivet-br:/br -v "C:/Users/you/brtar:/in:ro" \
#       rivet-card-build:1 tar -C /br -xf /in/br.tar
#   RIVET_BR_VOLUME=rivet-br tools/container/build-card.sh
#
# **It is a separate variable rather than magic in `RIVET_BR_OUTPUT`.** A value
# that is a path on one machine and a volume name on another is a value nobody
# can check; refusing to guess is the same rule the trust-key handling below
# already follows.
# **The volume is the default since 2026-08-30**, because Buildroot now runs in
# a container and writes there — `tools/container/build-buildroot.sh`. Before
# that it ran on whatever Linux host somebody had and left a directory behind,
# so a host path was the only sensible default; a card build that reached for
# one now finds nothing and says so a minute into the run.
#
# An explicit `RIVET_BR_OUTPUT` still wins, for the case this file already
# documents: a tree built somewhere else and mounted in.
if [ -n "${RIVET_BR_OUTPUT:-}" ]; then
    BR_VOLUME=${RIVET_BR_VOLUME:-}
else
    BR_VOLUME=${RIVET_BR_VOLUME:-rivet-br}
fi
BR_OUTPUT=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-$BR_VERSION}
if [ -n "$BR_VOLUME" ]; then
    "$DOCKER" volume inspect "$BR_VOLUME" >/dev/null 2>&1 \
        || die "RIVET_BR_VOLUME=$BR_VOLUME is not a docker volume. \`docker volume ls\` lists them."
elif [ ! -d "$BR_OUTPUT" ]; then
    cat >&2 <<EOF
tools/container/build-card.sh: no Buildroot output tree at
    $BR_OUTPUT

RIVET_BR_OUTPUT names it, or it is looked for beside \$HOME/.rivet by default.
It is NOT built in the container — the kernel, the cross toolchain and the C
mechanisms of ADR-009 §6 are hours of work and a separate question. Build one
with tools/buildroot/build.sh, or point RIVET_BR_OUTPUT at an existing tree.

If the tree is inside WSL, Windows Docker cannot mount it — a //wsl\$/… path
mounts EMPTY rather than failing. Use RIVET_BR_VOLUME; this file says how.
EOF
    exit 1
fi

# --- the inputs that are in the working tree but not in the commit ----------
#
# `cmd/rivet-image` reads the Raspberry Pi firmware from `vendor/boot`, which is
# gitignored: `ADR-010` §6 says the firmware is redistributable but is not this
# project's to carry. So a CLEAN CHECKOUT does not have it, and a container
# build of one fails several minutes in, after the whole workspace has
# compiled, with a message about a file rather than about a missing fetch step.
#
# Checked here because that is minutes earlier and one sentence clearer.
#
# **The kernel is NOT checked here.** A card boots Buildroot's kernel from
# `$RIVET_BR_OUTPUT/images`, which only the container can see, and
# `tools/card/build.sh` refuses early when it is absent. `tools/qemu/kernel8.img`
# is that script's accepted-degradation fallback, fetched off a Raspberry Pi OS
# board — requiring it here demanded a file a normal build never reads, from a
# board a new workstation may not have.
MISSING=""
for f in vendor/boot/start4.elf vendor/boot/fixup4.dat; do
    [ -e "$REPO/$f" ] || MISSING="$MISSING $f"
done
if [ -n "$MISSING" ]; then
    echo "tools/container/build-card.sh: the working tree is missing build inputs:" >&2
    for f in $MISSING; do echo "    $f" >&2; done
    cat >&2 <<'EOF'

These are gitignored on purpose (ADR-010 §6) and are fetched, not committed:

    tools/fetch-firmware.sh        writes vendor/boot

The container mounts the working tree, so what is missing here is missing
inside it too. Fetch them and run this again.
EOF
    exit 1
fi

OUT_DIR=${RIVET_OUT:-$REPO/out}
mkdir -p "$OUT_DIR"

# Read-only wherever the build only reads. The Buildroot tree in particular is
# expensive and shared; a build that could corrupt it would be a bad trade for
# nothing.
MOUNTS="-v $(hostpath "$REPO"):$C_REPO"
if [ -n "$BR_VOLUME" ]; then
    MOUNTS="$MOUNTS -v $BR_VOLUME:$C_BR:ro"
else
    MOUNTS="$MOUNTS -v $(hostpath "$BR_OUTPUT"):$C_BR:ro"
fi
MOUNTS="$MOUNTS -v $(hostpath "$OUT_DIR"):$C_OUT"

ENVS="-e RIVET_DESCRIBE=$RIVET_DESCRIBE -e RIVET_BR_OUTPUT=$C_BR"

NU=${RIVET_NU:-$HOME/.rivet/nushell/bin/nu}
if [ -f "$NU" ]; then
    MOUNTS="$MOUNTS -v $(hostpath "$NU"):$C_NU:ro"
    ENVS="$ENVS -e RIVET_NU=$C_NU"
else
    # Not fatal, and tools/card/build.sh says the same: a card with no nu falls
    # back to cmd/rivet as its login shell.
    echo "note: no nu at $NU — the card's login shell will be cmd/rivet"
    ENVS="$ENVS -e RIVET_NU=/nonexistent"
fi

if [ -n "${RIVET_DOCUMENT:-}" ]; then
    [ -f "$RIVET_DOCUMENT" ] || die "RIVET_DOCUMENT=$RIVET_DOCUMENT does not exist"
    # A single file, mounted read-only under a name of this script's choosing.
    # The host's filename is not carried in: it may say something about the
    # machine, and AGENTS.md §8.3 keeps that sort of thing out of build output.
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_DOCUMENT"):$C_DOC:ro"
    ENVS="$ENVS -e RIVET_DOCUMENT=$C_DOC"
fi

if [ -n "${RIVET_STAGING:-}" ]; then
    [ -f "$RIVET_STAGING" ] || die "RIVET_STAGING=$RIVET_STAGING does not exist"
    # Mounted read-only under a name of this script's choosing, for the reason
    # the document is: the host's filename is named after the machine.
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_STAGING"):$C_STAGING:ro"
    ENVS="$ENVS -e RIVET_STAGING=$C_STAGING"
fi

# **The carried payloads** -- ADR-050 §4. A space-separated list of .rvt paths
# on the HOST; each is mounted read-only under a name of this script's choosing
# and the list handed to tools/card/build.sh rewritten to the container's paths.
#
# Renamed for the document's reason and one more: the name on the CARD comes
# from the payload's signed header rather than from the file name (ADR-027 §5),
# so the host's filename carries nothing the build needs and may carry something
# about the machine (AGENTS.md §8.3).
if [ -n "${RIVET_APPLICATIONS:-}" ]; then
    C_APPS=""
    _n=0
    for _app in $RIVET_APPLICATIONS; do
        [ -f "$_app" ] || die "RIVET_APPLICATIONS names $_app and there is no file at that path"
        _n=$((_n + 1))
        _at="/payloads/app-$_n.rvt"
        MOUNTS="$MOUNTS -v $(hostpath "$_app"):$_at:ro"
        C_APPS="$C_APPS $_at"
    done
    # Leading space trimmed, so the variable the container sees is exactly the
    # list and `for` in the inner script does not see an empty first word.
    ENVS="$ENVS -e RIVET_APPLICATIONS=${C_APPS# }"
fi

if [ -n "${RIVET_IDENTITY:-}" ]; then
    [ -d "$RIVET_IDENTITY" ] || die "RIVET_IDENTITY=$RIVET_IDENTITY is not a directory"
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_IDENTITY"):$C_IDENTITY:ro"
    ENVS="$ENVS -e RIVET_IDENTITY=$C_IDENTITY"
fi

# **The fleet repository** -- ADR-050 §5.1. A host DIRECTORY, mounted read-only
# on RIVET_IDENTITY's own footing rather than RIVET_APPLICATIONS's per-file
# rewrite: tools/card/build.sh hands it straight to cmd/rivet-image, which
# reads whichever objects the composed document's role resolves to, by digest
# -- there is no fixed list of filenames this script could rename ahead of
# time the way it does for a named .rvt.
if [ -n "${RIVET_FLEET_REPOSITORY:-}" ]; then
    [ -d "$RIVET_FLEET_REPOSITORY" ] || die "RIVET_FLEET_REPOSITORY=$RIVET_FLEET_REPOSITORY is not a directory"
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_FLEET_REPOSITORY"):$C_FLEET_REPO:ro"
    ENVS="$ENVS -e RIVET_FLEET_REPOSITORY=$C_FLEET_REPO"
fi

# The trust keys default to keys/*.pub inside the repository, which the mount
# above already reaches. A caller who points them elsewhere is refused rather
# than quietly ignored: silently building a card that trusts something other
# than what was asked for is exactly the failure ADR-028 §5.5.5 is about.
for var in RIVET_TRUSTED_KEY RIVET_APP_KEY; do
    eval "val=\${$var:-}"
    [ -n "$val" ] || continue
    if under_repo "$val"; then
        ENVS="$ENVS -e $var=$(repo_relative "$val")"
    else
        die "$var=$val is outside the repository, and only the repository is mounted.
    Put the key under $REPO/keys/, or unset it to use keys/rivet-system.pub
    and keys/rivet-app.pub."
    fi
done

for var in RIVET_KEEP RIVET_NO_TRUST; do
    eval "val=\${$var:-}"
    [ -n "$val" ] && ENVS="$ENVS -e $var=$val"
done

# --- who the container runs as ----------------------------------------------
#
# On Linux a bind mount carries the host's uids straight through, so a container
# running as root leaves root-owned files in `out/` and in the working tree —
# hostile, and exactly the kind of thing that makes people reach for `sudo`
# afterwards. Passing the caller's own uid avoids it.
#
# On macOS and Windows the runtime maps ownership itself and an explicit
# `--user` is at best redundant; on Docker Desktop for Mac it can make the
# bind mount unwritable. So it is Linux only.
#
# Nothing about the image depends on the uid: a `/system` tree gets root
# ownership from `mke2fs -d` being handed a `--owner=0` tar, not from the
# builder's identity (disk.rs's header).
USER_ARG=""
if [ "$UNAME" = Linux ] && [ -z "${RIVET_DOCKER_USER-}" ]; then
    USER_ARG="--user $(id -u):$(id -g)"
elif [ -n "${RIVET_DOCKER_USER-}" ]; then
    USER_ARG="--user $RIVET_DOCKER_USER"
fi

# Cargo's registry and the target directory live in named volumes rather than
# in the repository. Two reasons and the second is the important one: a bind
# mount on Windows and macOS is slow enough to dominate a build, and putting
# `target/` in the tree would have the container's artifacts and the host's
# fight over the same directory with different toolchain paths inside them.
CACHE_TAG=${RIVET_CACHE_TAG:-}
VOL_CARGO="rivet-card-cargo$CACHE_TAG"
VOL_TARGET="rivet-card-target$CACHE_TAG"
MOUNTS="$MOUNTS -v $VOL_CARGO:/cargo -v $VOL_TARGET:/target"

# HOME is set because cargo wants one and the uid above may not appear in
# /etc/passwd. TMPDIR because tools/card/build.sh stages through mktemp.
ENVS="$ENVS -e HOME=/tmp -e TMPDIR=/tmp"

# --- the image --------------------------------------------------------------

if ! command -v "${DOCKER%% *}" >/dev/null 2>&1; then
    cat >&2 <<EOF
tools/container/build-card.sh: no '${DOCKER%% *}' on PATH.

On macOS and Linux, install Docker (or set RIVET_DOCKER=podman). On Windows,
Docker Desktop puts docker.exe on the PATH Git Bash sees; if instead your
Docker lives inside a WSL distro, run this script from inside that distro —
it takes the Linux branch there and needs no changes.
EOF
    exit 1
fi

have_image() { $DOCKER image inspect "$IMAGE" >/dev/null 2>&1; }

if [ "$REBUILD" = yes ] || ! have_image; then
    echo "--- building $IMAGE (this happens once) ---"
    $DOCKER build $PLATFORM -t "$IMAGE" \
        -f "$(hostpath "$HERE/Dockerfile")" \
        "$(hostpath "$HERE")"
fi

# --- run --------------------------------------------------------------------

if [ "$MODE" = shell ]; then
    CMD="/bin/bash"
else
    # shellcheck disable=SC2086
    CMD="tools/card/build.sh$PASSTHROUGH"
fi

# Says which of the two /br sources is really in use. It named $BR_OUTPUT
# unconditionally until 2026-08-29, so with RIVET_BR_VOLUME set it reported a
# host path that was not mounted — a banner that lies about a mount is worse
# than no banner, and this one would have hidden the very confusion the volume
# support exists to end.
if [ -n "$BR_VOLUME" ]; then
    echo "--- $DOCKER run: $C_REPO from $REPO, $C_BR from docker volume $BR_VOLUME ---"
else
    echo "--- $DOCKER run: $C_REPO from $REPO, $C_BR from $BR_OUTPUT ---"
fi

if [ "$PRINT_ONLY" = yes ]; then
    # shellcheck disable=SC2086
    printf '%s run --rm %s -w %s %s %s %s /bin/sh -c %s\n' \
        "$DOCKER" "$USER_ARG" "$C_REPO" "$MOUNTS" "$ENVS" "$IMAGE" "'$CMD'"
    exit 0
fi

# No `-t`. A tty would need `winpty` under Git Bash for no gain — the build's
# output is a stream of lines either way — and an allocated tty merges stderr
# into stdout, which this build's careful separation of the two would lose.
#
# shellcheck disable=SC2086
exec $DOCKER run $PLATFORM --rm $USER_ARG -w "$C_REPO" $MOUNTS $ENVS "$IMAGE" /bin/sh -c "$CMD"
