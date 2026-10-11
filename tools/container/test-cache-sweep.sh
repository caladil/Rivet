#!/bin/sh
# Rehearse exec.sh's cache sweep: a volume Docker refuses to remove must not
# abort the command the caller actually asked exec.sh to run.
#
# Usage:
#   tools/container/test-cache-sweep.sh
#
# ## What this proves and how
#
# `tools/container/exec.sh` sweeps dead build-cache volumes on every
# invocation (`STATE.md` §2). Under `set -eu`, a bare `"$DOCKER" volume rm`
# failing used to take the WHOLE SCRIPT down with it — so a volume Docker
# declines to remove (in use, or held by another container) aborted whatever
# real command exec.sh was asked to run. That has aborted three real runs in
# one week.
#
# There is no committed harness for exec.sh's own shell logic short of a real
# Docker daemon, so this is one: a FAKE `docker` (written below, into a
# scratch directory) that reports one dead-looking cache volume and REFUSES to
# remove it, and a real invocation of `exec.sh` pointed at that fake through
# `RIVET_DOCKER`. What is asserted:
#
#   1. exec.sh's own command still runs to completion (a marker file appears).
#   2. exec.sh's exit code is the command's own sentinel, not a `set -e` abort.
#   3. exec.sh said it could not reclaim the volume, rather than staying
#      silent about one it left behind.
#
# **Nothing here touches a real Docker volume.** The fake `docker` never calls
# the real one; `RIVET_DOCKER` is the same override `build-card.sh` documents
# for `podman`, aimed at a script instead.
#
# `RIVET_REHEARSE_KEEP=1` keeps the scratch directory afterwards, on
# `tools/provision/rehearse-reset.sh`'s footing.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

WORK=${RIVET_REHEARSE_DIR:-$(mktemp -d 2>/dev/null || echo "/tmp/rivet-cache-sweep-test.$$")}
MARKER="$WORK/ran"
FAKE_DOCKER="$WORK/docker"
LOG="$WORK/exec.stderr"

FAILED=""
fail() {
    echo "  FAIL: $1" >&2
    FAILED="yes"
}
ok() { echo "  ok   $1"; }

cleanup() {
    if [ -z "$FAILED" ] && [ -z "${RIVET_REHEARSE_KEEP:-}" ]; then
        rm -rf "$WORK"
    else
        echo ""
        echo "  the artifacts are in $WORK"
    fi
}
trap cleanup EXIT

echo "test-cache-sweep.sh: working in $WORK"
echo ""

# --- the fake docker ---------------------------------------------------------
#
# Reports exactly one dead-looking cache volume — a tag no worktree of this
# repository would ever produce, so exec.sh's own liveness test marks it dead
# and tries to reclaim it — refuses to remove it (exit 1, standing in for
# "volume in use" or anything else Docker might decline for), answers `image
# inspect` so exec.sh never tries to BUILD the image, and otherwise records
# that it was asked to `run` something and exits with a sentinel this script
# can tell apart from a `set -e` abort.
cat >"$FAKE_DOCKER" <<'FAKE'
#!/bin/sh
case "$1" in
    volume)
        case "$2" in
            ls) echo "rivet-card-cargo-test-cache-sweep-dead"; exit 0 ;;
            rm) exit 1 ;;
        esac
        ;;
    image)
        [ "$2" = inspect ] && exit 0
        ;;
    run)
        touch "$FAKE_DOCKER_MARKER"
        exit 42
        ;;
esac
exit 0
FAKE
chmod +x "$FAKE_DOCKER"

echo "--- exec.sh with a cache volume the fake docker refuses to remove"
rm -f "$MARKER"
CODE=0
RIVET_DOCKER="$FAKE_DOCKER" FAKE_DOCKER_MARKER="$MARKER" \
    "$HERE/exec.sh" true >/dev/null 2>"$LOG" || CODE=$?

if [ -f "$MARKER" ]; then
    ok "exec.sh's own command still ran despite the failed volume removal"
else
    fail "exec.sh's command never ran — the failed volume removal took the script down (see $LOG)"
fi

if [ "$CODE" -eq 42 ]; then
    ok "exec.sh exited with the command's own status (42), not a set -e abort"
else
    fail "exec.sh exited $CODE, expected 42 — a set -e abort reports the failed command's own code, which is 1"
fi

if grep -q "could not reclaim rivet-card-cargo-test-cache-sweep-dead" "$LOG"; then
    ok "exec.sh reported the volume it could not reclaim rather than staying silent"
else
    fail "exec.sh did not report the volume it left behind (see $LOG)"
fi

echo ""
if [ -n "$FAILED" ]; then
    echo "test-cache-sweep.sh: FAIL"
    exit 1
fi
echo "test-cache-sweep.sh: PASS — a volume the sweep cannot reclaim is reported and left in"
echo "place; it does not abort the command exec.sh was asked to run."
