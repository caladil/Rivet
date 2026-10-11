#!/bin/sh
# Is the cold-agent test actually ready to run? Answers READY or NOT READY.
#
# Usage:
#   tools/coldagent/preflight.sh              # stage 1 — a fresh workspace
#   tools/coldagent/preflight.sh --stage 2    # stage 2 — stage 1's output is EXPECTED
#
# ## Why this is a script and not a checklist
#
# **A checklist is something the person about to skip it is the one reading.**
# The cold-agent test has now been set up twice and been contaminated twice, in
# different ways each time:
#
#   * before the second run, eighteen agent worktrees — full checkouts of this
#     repository at eighteen states, several with another agent's work in
#     progress — were sitting inside `.claude/worktrees/`, while the briefing
#     tells the agent to read the repository *"freely — all of it"*;
#   * and an analysis document by another agent was sitting in the workspace the
#     briefing calls the agent's own root.
#
# Both were found by looking, once, because somebody thought to look. The first
# run's own verdict was that it had *"succeeded partly by reverse-engineering …
# another agent's log records"*, so this is not a hypothetical failure mode: it
# is the measured one, and it recurs because agents keep creating worktrees and
# runs keep writing their output into the workspace.
#
# **What makes a result worthless is not that the workspace is dirty. It is that
# nobody knows whether it was.** This prints what it checked either way.
#
# ## What it deliberately does NOT check
#
# Whether the BOARD is in a sensible state. It is a shared development device
# (`.rulings/` R-119) and other sessions legitimately have things on it — the
# briefing says so now. A preflight that demanded an empty board would be
# demanding something the test is supposed to survive.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WORKSPACE=${RIVET_COLDAGENT_WORKSPACE:-$HOME/dev/scratchpad/Rivet}

STAGE=1
while [ $# -gt 0 ]; do
    case "$1" in
        --stage) STAGE=$2; shift ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

FAIL=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  NOT READY  %s\n' "$*"; FAIL=1; }
ok() { printf '  ok         %s\n' "$*"; }

echo "--- cold-agent preflight, stage $STAGE ---"
echo "    workspace: $WORKSPACE"
echo "    repository: $REPO"
echo

# --- 1. the workspace ---------------------------------------------------------
#
# Stage 1 wants nothing but what the round supplies. Stage 2 EXPECTS stage 1's
# output, because refactoring it is the task — so the same directory is a pass
# and a fail depending on which test is being run, and getting that backwards is
# the whole reason this takes a flag.
#
# **What "supplies" means is per round, and it used to be hardcoded.** The check
# named `todo` because the 2026-08-30 round's application was a ToDo list, and
# `BRIEFING-STAGE-02.md` because that round had two stages. A round with neither
# failed this check for having the wrong filenames rather than for being dirty,
# which is a readiness check reporting on itself.
#
# So the rule is what it was always meant to be: **the workspace holds the
# briefing, whatever the briefing hands the agent, and no working material from
# a previous run.** `RIVET_COLDAGENT_GIVEN` names the extra files a round
# supplies, space separated.

if [ ! -d "$WORKSPACE" ]; then
    bad "the workspace does not exist"
else
    GIVEN=${RIVET_COLDAGENT_GIVEN:-reference.png}
    KEEP="BRIEFING.md BRIEFING-STAGE-02.md todo $GIVEN"
    EXTRA=$(ls -A "$WORKSPACE" || true)
    for allowed in $KEEP; do
        EXTRA=$(printf '%s\n' "$EXTRA" | grep -v "^${allowed}\$" || true)
    done
    TODO_COUNT=$(ls -A "$WORKSPACE/todo" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$STAGE" = 1 ]; then
        [ -f "$WORKSPACE/BRIEFING.md" ] || bad "there is no BRIEFING.md in the workspace"
        for supplied in $GIVEN; do
            [ -e "$WORKSPACE/$supplied" ] || bad "the briefing supplies '$supplied' and it is not in the workspace"
        done
        [ -z "$EXTRA" ] || bad "stage 1 wants only the briefing and what it supplies; also present: $(echo "$EXTRA" | tr '\n' ' ')"
        [ "$TODO_COUNT" = 0 ] || bad "a todo/ from an earlier round holds $TODO_COUNT entries — a previous run's application"
        [ -f "$WORKSPACE/BRIEFING.md" ] && [ -z "$EXTRA" ] && [ "$TODO_COUNT" = 0 ] &&
            ok "workspace holds the briefing and what it supplies, and nothing else"
    else
        [ -f "$WORKSPACE/BRIEFING-STAGE-02.md" ] || bad "stage 2 needs BRIEFING-STAGE-02.md and it is not there"
        [ "$TODO_COUNT" -gt 0 ] || bad "stage 2 refactors stage 1's application and todo/ is empty — run stage 1 first"
        [ -f "$WORKSPACE/REPORT.md" ] && note "REPORT.md from stage 1 is present; that is EXPECTED at stage 2 and the briefing should say whether it may be read"
        [ -f "$WORKSPACE/BRIEFING-STAGE-02.md" ] && [ "$TODO_COUNT" -gt 0 ] && ok "workspace carries stage 1's output and the stage 2 briefing"
    fi
fi

# --- 2. other agents' checkouts ----------------------------------------------
#
# These come BACK. Every delegated pass creates one, so this is not a thing that
# gets fixed once — it is a thing that gets checked immediately before a run.

TREES=$(git -C "$REPO" worktree list | grep -c 'worktrees/agent-' || true)
if [ "$TREES" != 0 ]; then
    bad "$TREES agent worktree(s) inside the repository — full checkouts the agent is told it may read"
    note "           git -C $REPO worktree list"
    note "           remove them, and their branches, before running"
else
    ok "no agent worktrees in the repository"
fi

DIRT=$(git -C "$REPO" status --porcelain | wc -l | tr -d ' ')
if [ "$DIRT" != 0 ]; then
    bad "$DIRT uncommitted change(s) in the repository — the agent reads a tree nobody can name"
else
    ok "repository tree is clean at $(git -C "$REPO" rev-parse --short HEAD)"
fi

# --- 3. the previous run's transcript ----------------------------------------
#
# This one is a WARNING and not a failure, and the distinction matters: the
# transcript does not load on its own. It only reaches a new agent if the run is
# started with --continue or --resume, which is an operator's mistake this
# script cannot prevent and should not pretend to.
#
# **It counts more than one runner's transcripts, because more than one runner
# has now been used.** The 2026-08-30 run was driven by Codex rather than Claude
# Code, and this section as first written looked only in
# `~/.claude/projects/`. It reported a clean workspace while a five-hour
# transcript of that very exercise sat in `~/.codex/sessions/`, unseen. A
# check that knows about one vendor is a check that silently stops working the
# day somebody uses another.

CLAUDE_SESSIONS=$HOME/.claude/projects/C--Users-$(basename "$HOME")-dev-scratchpad-Rivet
CLAUDE_N=$(ls "$CLAUDE_SESSIONS" 2>/dev/null | wc -l | tr -d ' ')
CODEX_N=$(find "$HOME/.codex/sessions" -name 'rollout-*.jsonl' 2>/dev/null | wc -l | tr -d ' ')
TOTAL=$((CLAUDE_N + CODEX_N))
if [ "$TOTAL" != 0 ]; then
    note "note       $TOTAL previous session transcript(s): $CLAUDE_N Claude Code, $CODEX_N Codex."
    note '           They do NOT load on their own. START A NEW SESSION: --continue or'
    note '           --resume would hand the next agent every finding the last one made.'
    [ "$CODEX_N" != 0 ] && note '           Codex rollouts are not per-directory, so that count is every'
    [ "$CODEX_N" != 0 ] && note '           Codex thread on this machine, not only ones for the workspace.'
fi

# --- 4. can the board be reached at all? -------------------------------------
#
# Not whether it is tidy — whether the test is possible. A run that fails
# because the machine is off is a wasted run and an unfair result.

BOARD=${RIVET_BOARD:-}
if [ -z "$BOARD" ]; then
    note "note       set RIVET_BOARD=<host:port> to have this check the machine answers."
else
    # bash's /dev/tcp, NOT `nc`. There is no `nc` in this project's WSL distro
    # and none in Git Bash either — a wait loop built on it once spun for 6h40m
    # because it could never succeed. The shell has the probe built in and it
    # needs nothing installed.
    #
    # **And an unavailable probe must not print `ok`.** The first version of
    # this said "ok skipped: no nc", which is a script whose whole purpose is
    # distinguishing checked from assumed, reporting an assumption as a check.
    if timeout 4 bash -c "exec 3<>/dev/tcp/${BOARD%:*}/${BOARD##*:}" 2>/dev/null; then
        ok "the board answers on $BOARD"
    else
        bad "nothing answers on $BOARD — the test cannot run and a run that fails
             because the machine is off is a wasted run and an unfair result"
    fi
fi

echo
if [ "$FAIL" = 0 ]; then
    echo "READY — everything above was checked, not assumed."
    exit 0
fi
echo "NOT READY — fix the lines above. Nothing was changed by this script."
exit 1
