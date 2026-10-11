#!/bin/sh
# Run one command in the build container, with everything a build needs mounted.
#
# `tools/container/build-card.sh` and `build-update.sh` are wrappers around ONE
# recipe each. This is the general one: it mounts the repository, the Buildroot
# tree and the caches, and runs whatever it is given.
#
#   tools/container/exec.sh examples/probes/libgreet/build.sh
#   tools/container/exec.sh cargo test --workspace
#   tools/container/exec.sh sh            # look around
#
# ## Why it exists
#
# `ADR-026` §7 made this project Docker rather than WSL, and the two build
# wrappers were the only ways in. Anything else — running the test suite on the
# Linux crates, building a C library for the target, reading a cross-compiled
# binary with `readelf` — meant either a hand-written `docker run` in somebody's
# scrollback or a distro nobody else has. Both are the thing `ADR-026` §6 argues
# against.
#
# ## The signing seed
#
# `RIVET_APP_SEED` names one on the HOST; it is mounted read-only for the length
# of one command, exactly as `build-update.sh` mounts the system seed and for
# the reason argued there: a read-only bind mount hands a file already on this
# machine to a process on the SAME machine, under the same user. Nothing is
# baked into an image and nothing reaches a registry.
#
# Without it, `cargo rivet library` and `cargo rivet package` refuse — and the
# refusal names all three places they look, which is the correct behaviour for a
# tool that will not guess which key you meant to sign with.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
DOCKER=${RIVET_DOCKER:-docker}
# The platform every image here is built and run as. The pinned Arch base and
# Buildroot's Bootlin toolchain exist only for x86-64, so an arm64 host (Apple
# Silicon) runs them emulated; ADR-062 §4. On an x86-64 host this is a no-op.
PLATFORM="--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}"
# **The image tag, read from ONE file** - `tools/container/image-version`,
# and see the `.md` beside it for why. Four scripts spelled this number
# until 2026-09-02, which is four places a bump can miss one.
# `tr -d` because a checkout can hand this back with a CRLF whatever
# `.gitattributes` says - proven on 2026-09-02, when seven tracked scripts
# were found defying `*.sh text eol=lf`. A trailing \r here would make the
# tag `rivet-card-build:3\r`, which matches no image.
IMAGE_VERSION=$(tr -d '\r\n' <"$HERE/image-version")
IMAGE=rivet-card-build:$IMAGE_VERSION

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

# See tools/container/build-card.sh for why both halves of this are needed.
hostpath() {
    if [ "$IS_MSYS" = yes ]; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

BR_VOLUME=${RIVET_BR_VOLUME:-rivet-br}

# --- reclaiming the per-worktree caches -------------------------------------
#
# **The per-worktree tag below solves one problem and creates a smaller one.**
# Each linked worktree gets its own `/cargo` and `/target` volume, and a Docker
# volume outlives the directory it was named after — so a worktree that is
# removed leaves multiple gigabytes behind with nothing pointing at it.
#
# Measured 2026-09-04, the day the tag was introduced: four finished agent
# worktrees had left **38 GB** of caches on a disk that was 100% full. That is
# the recurrence, and it is this script's to close rather than the operator's to
# remember.
#
# A volume is dead when no worktree of this repository would produce its tag.
# `git worktree list` is the authority, and a volume is only ever removed when
# it matches this repository's own naming — nothing here can reach
# `rivet-br`, the buildroot source, or another project's caches.
# **The tags a worktree of this repository would produce**, one per line, by the
# same reduction the tag itself is built with below. Two copies of that
# expression would be a way for a volume to be named one thing and looked for
# under another.
#
# **A listing that fails is a refusal, never an empty answer.** This script
# exports `MSYS_NO_PATHCONV=1` for Docker's sake, which stops Git Bash
# translating `/d/...` for Windows `git` — so `git -C "$REPO"` failed, its error
# went to /dev/null, the live list came back empty, and every worktree's cache
# read as dead. Only Docker refusing to remove a mounted volume saved the one in
# use. The path is handed over in the host's own form, and a failed listing
# returns non-zero so the caller sweeps nothing.
live_tags() {
    _listing=$(git -C "$(hostpath "$REPO")" worktree list --porcelain) || return 1
    printf '%s\n' "$_listing" \
        | sed -n 's/^worktree //p' \
        | while read -r w; do basename "$w" | tr -d '\r\n' | tr -c 'A-Za-z0-9_.-' '-' | cut -c1-40; echo; done
}

# **Cache volumes no worktree would produce the tag for**, one per line.
#
# `$1` is a tag to spare, with its leading `-`, and it exists for the one case
# the liveness test cannot see: a tag set by hand has no directory behind it, so
# it looks exactly like a removed worktree's. The caller passes the tag it is
# about to mount, which is the only hand-set tag this process can know about.
dead_cache_volumes() {
    _spare=${1:-}
    _live=$(live_tags) || {
        echo "exec.sh: could not list this repository's worktrees, so no cache was reclaimed." >&2
        return 0
    }
    for _vol in $("$DOCKER" volume ls --format '{{.Name}}' 2>/dev/null \
        | grep -E '^rivet-card-(cargo|target)-' | sort); do
        _tag=${_vol#rivet-card-cargo-}
        _tag=${_tag#rivet-card-target-}
        if [ -n "$_spare" ] && [ "-$_tag" = "$_spare" ]; then continue; fi
        if printf '%s\n' "$_live" | grep -qx "$_tag"; then continue; fi
        printf '%s\n' "$_vol"
    done
}

if [ "${1:-}" = "--prune-worktree-caches" ]; then
    live=$(live_tags) || {
        echo "exec.sh: could not list this repository's worktrees; refusing to name anything dead." >&2
        exit 1
    }
    # **It LISTS by default and removes only with `--yes`.**
    #
    # A tag somebody set by hand with `RIVET_CACHE_TAG` is indistinguishable
    # from a removed worktree's — both are just a suffix with no directory
    # behind them. So this cannot tell "dead" from "in use by the session
    # running it", and a tool that guesses wrong here deletes the cache of the
    # work in progress. Printing the list costs one command and removes that
    # whole class of mistake.
    candidates=""
    for vol in $("$DOCKER" volume ls --format '{{.Name}}' 2>/dev/null \
        | grep -E '^rivet-card-(cargo|target)-' | sort); do
        tag=${vol#rivet-card-cargo-}
        tag=${tag#rivet-card-target-}
        if printf '%s\n' "$live" | grep -qx "$tag"; then
            printf 'keep    %-34s worktree %s is live\n' "$vol" "$tag" >&2
            continue
        fi
        candidates="$candidates $vol"
    done

    if [ -z "$candidates" ]; then
        echo "exec.sh: nothing to reclaim — every cache belongs to a live worktree." >&2
        exit 0
    fi

    if [ "${2:-}" != "--yes" ]; then
        echo "" >&2
        echo "exec.sh: these caches match no live worktree:" >&2
        for vol in $candidates; do
            printf '        %-34s %s\n' "$vol" \
                "$("$DOCKER" system df -v 2>/dev/null | awk -v v="$vol" '$1==v {print $NF}')" >&2
        done
        echo "" >&2
        echo "  A tag set by hand with RIVET_CACHE_TAG looks exactly like a removed" >&2
        echo "  worktree's, so READ THE LIST before acting — the cache of the session" >&2
        echo "  you are running in can be on it. Then:" >&2
        echo "" >&2
        echo "      tools/container/exec.sh --prune-worktree-caches --yes" >&2
        exit 0
    fi

    removed=0
    for vol in $candidates; do
        printf 'remove  %s\n' "$vol" >&2
        "$DOCKER" volume rm "$vol" >/dev/null 2>&1 && removed=$((removed + 1))
    done
    echo "exec.sh: removed $removed cache volume(s). The primary checkout's untagged" >&2
    echo "         caches, rivet-br and the buildroot source are never touched." >&2
    exit 0
fi

# ## A LINKED WORKTREE GETS ITS OWN BUILD CACHE, AND MUST
#
# `/cargo` and `/target` are NAMED DOCKER VOLUMES, shared by every container
# this script ever starts. The repository is mounted at `/rivet` — the same
# container path every time, whichever checkout it came from. Those two facts
# together are a trap with no error message:
#
# **cargo fingerprints a crate by its path, and every tree's path is
# `/rivet/...`.** Two checkouts of this repository are therefore
# indistinguishable to cargo, and a build from one is silently reused as a build
# of the other. Not a stale-cache warning, not a rebuild — the wrong binary,
# run and reported as yours.
#
# Found on 2026-09-04 with two subagents working in `.claude/worktrees/` while
# the lead session ran the suite. `cargo test -p aura-card` in the lead's tree
# ran two test functions that **did not exist in that tree**, at HEAD or in the
# working file, and reported them as failures of it. Under an isolated tag the
# same crate answered 38 tests, all passing. A false PASS is available by the
# same route and would be worse, because nothing about it looks wrong.
#
# So: a **linked worktree** — one whose `.git` is a file pointing elsewhere,
# which is exactly the concurrent case — gets a tag derived from its own
# directory name. The primary checkout keeps the untagged volumes it has always
# used, so no existing cache is invalidated and nobody pays a rebuild for a
# hazard they were not exposed to. `RIVET_CACHE_TAG` still overrides both.
#
# The cost is a cold first build per worktree, which is the honest price of an
# answer that is about the tree you are in.
if [ -n "${RIVET_CACHE_TAG+set}" ]; then
    CACHE_TAG=$RIVET_CACHE_TAG
elif [ -f "$REPO/.git" ]; then
    # A worktree name can carry anything a directory name can; reduce it to
    # what a Docker volume name accepts rather than letting `docker run` refuse
    # with a message about the volume and not about this.
    # `tr -d` before `tr -c`, or `basename`'s own newline becomes a trailing
    # `-` in the volume name — harmless, and the kind of detail that makes two
    # people reading the same tag disagree about whether they are on it.
    CACHE_TAG=-$(basename "$REPO" | tr -d '\r\n' | tr -c 'A-Za-z0-9_.-' '-' | cut -c1-40)
    echo "exec.sh: linked worktree — using build cache '$CACHE_TAG' so this tree is not" >&2
    echo "         built out of another one's artifacts. Set RIVET_CACHE_TAG to override." >&2
    echo "         It is MULTI-GIGABYTE and outlives the worktree:" >&2
    echo "         tools/container/exec.sh --prune-worktree-caches  reclaims dead ones." >&2
else
    CACHE_TAG=
fi

# ## THE SWEEP RUNS ITSELF, NOW THAT EVERY TAG HAS A DIRECTORY BEHIND IT
#
# **A reclaimer that runs only when somebody remembers it is a reclaimer that
# does not run.** `--prune-worktree-caches` above has existed since 2026-09-04
# and was correct every day since; on 2026-09-12 the volumes it would have
# removed came to **241 GB across 84 tags**, on a system drive with 7.5 GB left.
# It had not refused through any fault of its own — nobody had typed it.
#
# It could not be made automatic before, and the reason is the one its own
# comments give: a tag set by hand has no directory behind it and is
# indistinguishable from a removed worktree's, so a sweep that ran unasked could
# delete the cache of the session running it. `STATE.md` told every agent to set
# one by hand, so *most* tags were of exactly that kind.
#
# That instruction is gone. Isolation now comes from working in a worktree,
# which is where it should have come from — `git worktree list` is then the
# authority on what is live, the test is exact rather than merely usually right,
# and the sweep is safe unasked. The single tag with no directory behind it is
# this invocation's own, and that is precisely what is spared.
#
# `RIVET_NO_CACHE_SWEEP=1` turns it off, for the case somebody is deliberately
# holding a hand-tagged cache across sessions and knows they are.
#
# **A removal that fails must not abort the command this script was actually
# asked to run.** Under `set -eu`, a bare `"$DOCKER" volume rm` exiting non-zero
# used to take the whole script down with it — so a volume Docker cannot remove
# right now (in use, or held by another container) turned a routine build or
# test run into a failure with nothing to do with what was asked for. This has
# aborted three real runs in one week. The sweep is best-effort tidying, not
# part of the command's own contract: report the miss and move on, leaving the
# volume for a later sweep to try again.
if [ -z "${RIVET_NO_CACHE_SWEEP:-}" ]; then
    for vol in $(dead_cache_volumes "$CACHE_TAG"); do
        echo "exec.sh: reclaiming $vol — no worktree of this repository names it." >&2
        if ! "$DOCKER" volume rm "$vol" >/dev/null 2>&1; then
            echo "exec.sh: could not reclaim $vol — left in place" >&2
        fi
    done
fi

MOUNTS="-v $(hostpath "$REPO"):/rivet"
MOUNTS="$MOUNTS -v $BR_VOLUME:/br:ro"
MOUNTS="$MOUNTS -v rivet-card-cargo$CACHE_TAG:/cargo -v rivet-card-target$CACHE_TAG:/target"

ENVS="-e HOME=/tmp -e TMPDIR=/tmp -e RIVET_BR_OUTPUT=/br"

# The application signing seed, if the caller has one and named it.
if [ -n "${RIVET_APP_SEED:-}" ]; then
    [ -f "$RIVET_APP_SEED" ] || {
        echo "tools/container/exec.sh: RIVET_APP_SEED=$RIVET_APP_SEED does not exist" >&2
        exit 1
    }
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_APP_SEED"):/seed/app.seed:ro"
    ENVS="$ENVS -e RIVET_APP_SEED=/seed/app.seed"
fi

# The system signing seed, on the same terms.
#
# **Added 2026-09-01, and it should have been here from the start.** The
# application seed was mounted and this one was not, so `tools/update/build.sh`
# -- a committed recipe, written precisely so an update is not built out of
# somebody's scrollback -- could not run in the container that runs everything
# else. The two seeds are kept apart on purpose (`ADR-029`); mounting them is
# the same problem twice and there is no argument for solving it once.
if [ -n "${RIVET_SYSTEM_SEED:-}" ]; then
    [ -f "$RIVET_SYSTEM_SEED" ] || {
        echo "tools/container/exec.sh: RIVET_SYSTEM_SEED=$RIVET_SYSTEM_SEED does not exist" >&2
        exit 1
    }
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_SYSTEM_SEED"):/seed/system.seed:ro"
    ENVS="$ENVS -e RIVET_SYSTEM_SEED=/seed/system.seed"
fi

# **Another project's source tree, READ-ONLY** — `PLAN.md` §104.25.
#
# Aura is a separate repository and stays one. Its C++ renderer has to be built
# for this target, and the build that does it lives here — in
# `deploy/aura/renderer/` — because it is a RIVET build: Raylib's DRM backend,
# the Buildroot cross toolchain, and the eight shared libraries a Rivet card
# carries. None of that is Aura's business and none of it belongs in Aura's
# tree.
#
# **Read-only, and named rather than guessed.** The same argument
# `RIVET_APP_SEED` makes above: a bind mount hands a directory already on this
# machine to a process on the same machine, for the length of one command. What
# it must not do is let a build write into somebody else's repository, so it
# cannot.
if [ -n "${RIVET_AURA_SRC:-}" ]; then
    [ -d "$RIVET_AURA_SRC" ] || {
        echo "tools/container/exec.sh: RIVET_AURA_SRC=$RIVET_AURA_SRC is not a directory" >&2
        exit 1
    }
    [ -f "$RIVET_AURA_SRC/cpp/src/main.cpp" ] || {
        echo "tools/container/exec.sh: $RIVET_AURA_SRC has no cpp/src/main.cpp, so it is not" >&2
        echo "    an Aura checkout. Refusing rather than mounting the wrong tree and" >&2
        echo "    failing later with a compiler error about a missing file." >&2
        exit 1
    }
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_AURA_SRC"):/aura:ro"
    ENVS="$ENVS -e RIVET_AURA_SRC=/aura"
fi

# **An APPLICATION's source tree, beside this one, WRITABLE** — `ADR-040` §10,
# added 2026-09-05 for the Trends port.
#
# `RIVET_AURA_SRC` above mounts another project read-only, and that is right for
# Aura: this repository holds the build and Aura holds only the source. **An SDK
# adopter is the other shape.** Trends and TempMon own their own build, their own
# `Cargo.lock` and their own target directory, and every one of those is a write.
# A read-only mount would make `cargo build` fail on the lock file.
#
# ## The mount points are not arbitrary and cannot be
#
# `rivetos-sdk` is unpublished and this repository is untagged, so an adopter has
# no git dependency to pin and reaches the SDK **by relative path to a sibling
# checkout** — `path = "../../../Rivet/rivet-sdk"` in Trends' case. That path
# resolves only if the two trees are siblings and this one is called `Rivet`.
#
# So both are mounted under `/src`, with **this repository at `/src/Rivet`
# whatever the checkout is called** — a linked worktree is named after the agent
# that made it, and an adopter's manifest cannot know that. The adopter keeps its
# own directory name, because its manifest's `../../../Rivet` counts levels from
# where its own crate sits.
#
# `AGENTS.md` §8.4 forbids a sibling path dependency in general and is right; the
# adopter's manifest says so about itself. **When this repository cuts a tag,
# every adopter becomes a git dependency and this block stops being needed.**
if [ -n "${RIVET_APP_SRC:-}" ]; then
    [ -d "$RIVET_APP_SRC" ] || {
        echo "tools/container/exec.sh: RIVET_APP_SRC=$RIVET_APP_SRC is not a directory" >&2
        exit 1
    }
    APP_NAME=$(basename "$(cd "$RIVET_APP_SRC" && pwd)" | tr -d '\r\n')
    [ "$APP_NAME" != Rivet ] || {
        echo "tools/container/exec.sh: RIVET_APP_SRC is called 'Rivet', which is the name this" >&2
        echo "    repository is mounted under. Refusing rather than shadowing it." >&2
        exit 1
    }
    MOUNTS="$MOUNTS -v $(hostpath "$REPO"):/src/Rivet"
    MOUNTS="$MOUNTS -v $(hostpath "$RIVET_APP_SRC"):/src/$APP_NAME"
    ENVS="$ENVS -e RIVET_APP_SRC=/src/$APP_NAME -e RIVET_REPO=/src/Rivet"
fi

USER_ARG=""
if [ "$UNAME" = Linux ] && [ -z "${RIVET_DOCKER_USER-}" ]; then
    USER_ARG="--user $(id -u):$(id -g)"
fi

[ $# -gt 0 ] || set -- sh

# **Build the image if this machine does not have it** — the same block
# `build-card.sh` has carried since the image existed, added here 2026-09-03.
#
# `image-version.md` says *"the scripts rebuild automatically when the tag names
# an image that does not exist locally"*, and that sentence was true of three
# scripts and false of this one. The consequence was waiting for the next bump:
# `image-version` goes to 4, every other entry point rebuilds, and this one —
# the general way in, the one that runs the test suite — fails with
# `Unable to find image 'rivet-card-build:4'`. Found by bumping it.
have_image() { $DOCKER image inspect "$IMAGE" >/dev/null 2>&1; }

if ! have_image; then
    echo "--- building $IMAGE (this happens once) ---" >&2
    $DOCKER build $PLATFORM -t "$IMAGE" \
        -f "$(hostpath "$HERE/Dockerfile")" \
        "$(hostpath "$HERE")"
fi

# shellcheck disable=SC2086
exec $DOCKER run $PLATFORM --rm $USER_ARG -w /rivet $MOUNTS $ENVS "$IMAGE" "$@"
