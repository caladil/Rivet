#!/bin/sh
# Check `docs/COMMANDS.md` against the binaries' OWN `--help`, from inside the
# build container. Plan `PLAN-Feature-Set-2026-09-12.md` §9.1 asks for the
# command reference to be "checked by the suite"; `tools/test-all.sh` runs
# this, not `tools/docs/commands.py`, because the container has neither `uv`
# nor a Python interpreter (checked 2026-09-13: `exec: "uv": executable file
# not found`, and the same for `python3`) — so the thing that GATES a
# container run cannot need either.
#
# `commands.py` (run with `uv run` from a workstation) is still the one
# GENERATOR — it writes `docs/COMMANDS.md` in the first place, from the same
# `--help` output this script checks against. This script never writes
# anything; it fails by naming the binary whose live `--help` no longer
# matches what is committed, which is the same signal `--bless` responds to
# on the generator side.
#
# Both tools read `tools/docs/commands-manifest.tsv` — the one list of which
# binaries are generated and why the rest are not — so neither can drift into
# naming a different set of binaries than the other.
#
#   sh tools/docs/check-commands.sh
#
# Exits 0 and prints nothing but a summary line when everything matches;
# exits 1 and names every mismatch otherwise.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
cd "$REPO"

MANIFEST="tools/docs/commands-manifest.tsv"
COMMANDS="docs/COMMANDS.md"

[ -f "$MANIFEST" ] || { echo "check-commands.sh: $MANIFEST is missing" >&2; exit 1; }
[ -f "$COMMANDS" ] || { echo "check-commands.sh: $COMMANDS is missing — run \`uv run tools/docs/commands.py\` first" >&2; exit 1; }

# Only the included binaries need a fresh build; `rivet-launcher` (the one
# excluded binary that would otherwise need building) is skipped by never
# being named.
NAMES=$(grep -v '^#' "$MANIFEST" | grep -v '^[[:space:]]*$' | awk -F'\t' '$3 == "yes" {print $1}')

[ -n "$NAMES" ] || { echo "check-commands.sh: $MANIFEST names no included binary — refusing to pass on nothing checked" >&2; exit 1; }

# `--exclude rivetos-launcher` rather than naming every included binary with
# `--bin`: `rivetos-launcher` is the one workspace member that cannot link
# for the container's own architecture (`commands-manifest.tsv`'s row says
# why), and building everything else the ordinary way is what every other
# check in this suite already does — a second, narrower build recipe here
# would be one more thing to keep in step with the workspace's membership.
echo "check-commands.sh: building the workspace's binaries (except rivetos-launcher)..."
cargo build --workspace --bins --exclude rivetos-launcher >/dev/null

FAILED=0
CHECKED=0
for name in $NAMES; do
    bin="target/debug/$name"
    [ -x "$bin" ] || bin="/target/debug/$name"
    [ -x "$bin" ] || {
        echo "MISMATCH $name: not built at target/debug/$name or /target/debug/$name" >&2
        FAILED=1
        continue
    }

    # Combine stdout and stderr in the SAME shell that runs the binary, for
    # the reason `commands.py`'s own comment gives: only redirecting at the
    # source preserves the true interleaving of a binary that writes to both.
    live=$("$bin" --help 2>&1 || true)
    # A version line's parenthesised commit becomes `(<commit>)`, exactly as
    # `commands.py`'s `normalise_version` does: the commit says which checkout
    # built the binary, so leaving it in would fail this check on every commit.
    live=$(printf '%s\n' "$live" | sed -E 's/^([^ ]+ [0-9]+\.[0-9]+\.[0-9]+[^ ]*) \([^)]*\)$/\1 (<commit>)/')

    # The fenced block immediately under `### \`name\`` (with or without the
    # generator's " — exits N against \`--help\`" suffix). This is the ONLY
    # place either tool encodes the heading shape; if `commands.py` changes
    # it, this line changes with it in the same commit.
    #
    # `sed 's/\r$//'` first: Windows `git` checks a tracked `.md` out as CRLF
    # even though the committed blob is LF (`AGENTS.md` §10.5 — no `.gitattributes`
    # pin forces `eol=lf` for prose, only for what a shell or kconfig parses
    # byte-strictly). The bind-mounted working tree this container sees is the
    # CRLF copy, so a bare `$0 == "```"` never matches a fence line ending
    # `` ` \r ``, and the whole extraction reads as empty — found once, here,
    # against `femto`, before this line existed.
    expected=$(sed 's/\r$//' "$COMMANDS" | awk -v name="$name" '
        $0 ~ ("^### `" name "`") { found = 1; next }
        found && !infence && $0 == "```" { infence = 1; next }
        found && infence && $0 == "```" { exit }
        found && infence { print }
    ')

    CHECKED=$((CHECKED + 1))
    if [ "$live" != "$expected" ]; then
        echo "MISMATCH $name: live --help does not match docs/COMMANDS.md — regenerate with \`uv run tools/docs/commands.py\`" >&2
        FAILED=1
    fi
done

if [ "$FAILED" -ne 0 ]; then
    echo "check-commands.sh: FAIL — $CHECKED binaries checked, at least one mismatch" >&2
    exit 1
fi
echo "check-commands.sh: docs/COMMANDS.md matches $CHECKED binaries' live --help"
