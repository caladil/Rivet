#!/bin/sh
# Increment the patch version and commit it, alone — `ADR-027` §10.
#
# **Run this on the WINDOWS side, from Git Bash. Not under WSL.** It refuses to
# run there, and the refusal is the whole reason this is a separate script from
# `tools/card/build.sh`.
#
# ## Why the bump cannot live in build.sh
#
# `ADR-027` §10.2 rules the sequence: **refuse a dirty tree → increment the
# patch → commit that one change alone → build.** The first three steps need
# `git`, and `AGENTS.md` §10.5 records that this repository has two of them that
# disagree: Windows `git` runs `core.autocrlf=true` and reads the tree as clean,
# WSL's has it off and reads every text file as modified.
#
# That makes both ends of the sequence wrong under WSL:
#
#   * **the refusal** would fire on every invocation, because WSL's `git status`
#     is never clean here; and
#   * **the commit** would write CRLF blobs — `.gitattributes` pins `*.sh` and
#     the kconfig fragments, but nothing pins `Cargo.toml` or `Cargo.lock`.
#
# `build.sh` runs under WSL because `sgdisk`, `mkfs` and `mtools` live only in
# the `archlinux` distro. So the sequence splits at the line the environment
# already draws: **this script decides the version, `build.sh` reads it.**
#
# ## Usage
#
#   tools/card/bump.sh            # bump, commit, print the new describe
#   tools/card/bump.sh --dry-run  # say what it would do and change nothing
#
# It prints the describe string on stdout as its last line, which is what
# `build.sh` wants:
#
#   RIVET_DESCRIBE=$(tools/card/bump.sh | tail -1)
#
# ## Why the bump is committed BEFORE the build
#
# `ADR-027` §10.2, and it is worth restating because the other order looks
# tidier: a bump that is not committed leaves every image stamped `-dirty` for
# ever, and lets two different images carry one version. Committing first makes
# **version and commit a permanent 1:1 mapping**. Bumping after a successful
# build — so failed builds do not consume numbers — was considered and refused
# there: the artifact would carry the old version while being named for the new.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
cd "$REPO"

DRY=
[ "$1" = "--dry-run" ] && DRY=1

# --- the refusal that gives this script its reason to exist ------------------
if [ -n "$WSL_DISTRO_NAME" ] || grep -qi 'microsoft' /proc/sys/kernel/osrelease 2>/dev/null; then
    cat >&2 <<EOF
tools/card/bump.sh: this is running under WSL, and it must not.

WSL's git has core.autocrlf off and reads this whole working copy as modified
(AGENTS.md §10.5). The clean-tree check below would refuse every time, and a
commit made here would write CRLF blobs for Cargo.toml and Cargo.lock, which
nothing in .gitattributes pins.

Run it from Git Bash on the Windows side, then hand the result to build.sh:

    RIVET_DESCRIBE=\$(tools/card/bump.sh | tail -1)
EOF
    exit 2
fi

# --- refuse a dirty tree (ADR-027 §10.2, step 1) -----------------------------
if [ -n "$(git status --porcelain)" ]; then
    cat >&2 <<EOF
tools/card/bump.sh: the tree is dirty and ADR-027 §10.2 refuses to build from it.

The version and the commit are a 1:1 mapping, and they can only be that if the
number is minted on a tree with nothing else in it. Commit or stash first:

$(git status --short | sed 's/^/    /')
EOF
    exit 1
fi

# --- increment the patch (step 2) --------------------------------------------
#
# ADR-027 §10: Cargo.toml's [workspace.package] version is the SINGLE source.
# Everything else reads it; nothing else states it.
CURRENT=$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
case "$CURRENT" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *)
        echo "tools/card/bump.sh: no [workspace.package] version in Cargo.toml (read '$CURRENT')" >&2
        exit 1
        ;;
esac

MAJOR=${CURRENT%%.*}
REST=${CURRENT#*.}
MINOR=${REST%%.*}
PATCH=${REST#*.}
NEXT="$MAJOR.$MINOR.$((PATCH + 1))"

echo "version: $CURRENT -> $NEXT"

# --- which packages carry the workspace version ------------------------------
#
# Read off the ROOT Cargo.lock rather than guessed: a package with no `source`
# line is a path package (a member of this workspace), and one whose version is
# the workspace's is one that INHERITS it. That excludes the two members that
# state their own -- `ixr-smb-server` at 0.5.1 and `rivetos-smbd` at 0.1.0 --
# without naming either, so the list does not need maintaining.
#
# Computed BEFORE the edit, while the lock still says $CURRENT.
WORKSPACE_PKGS=$(awk -v cur="$CURRENT" '
    /^\[\[package\]\]/ { nm=""; ver=""; src=0; next }
    /^name = /    { nm=$0;  sub(/^name = "/,    "", nm);  sub(/"\r?$/, "", nm);  next }
    /^version = "/{ ver=$0; sub(/^version = "/, "", ver); sub(/"\r?$/, "", ver); next }
    /^source = /  { src=1; next }
    /^\r?$/       { if (nm != "" && src == 0 && ver == cur) printf "%s ", nm
                    nm=""; ver=""; src=0 }
    END           { if (nm != "" && src == 0 && ver == cur) printf "%s ", nm }
' Cargo.lock)

if [ -z "$WORKSPACE_PKGS" ]; then
    echo "tools/card/bump.sh: no package in Cargo.lock carries version $CURRENT." >&2
    echo "  The lock is out of step with Cargo.toml; run 'cargo metadata' and commit it." >&2
    exit 1
fi

# Every tracked Cargo.lock except the root one. `git ls-files` rather than a
# find, because an untracked lock in somebody's scratch directory must not be
# swept into the version commit.
NESTED_LOCKS=$(git ls-files '*Cargo.lock' | grep -v '^Cargo\.lock$' || true)

if [ -n "$DRY" ]; then
    echo "--dry-run: nothing changed"
    echo "--dry-run: would refresh these nested locks:"
    for lock in $NESTED_LOCKS; do
        for pkg in $WORKSPACE_PKGS; do
            if grep -q "^name = \"$pkg\"" "$lock" 2>/dev/null; then
                echo "    $lock"
                break
            fi
        done
    done
    git describe --tags --always --dirty
    exit 0
fi

# Only the line inside [workspace.package]. A crate that states its own version
# rather than inheriting one would be a second source of the fact and ADR-011 §3
# is about exactly that, so this deliberately does not go looking for others.
# `-i.bak`, then the backup removed: GNU sed takes `-i` alone and BSD sed (macOS)
# reads the next word as a backup suffix, so a bare `-i` is not portable and
# `-i.bak` is the one spelling both accept.
sed -i.bak '/^\[workspace\.package\]/,/^\[/ s/^version = "'"$CURRENT"'"$/version = "'"$NEXT"'"/' Cargo.toml
rm -f Cargo.toml.bak

CHECK=$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
if [ "$CHECK" != "$NEXT" ]; then
    git checkout -- Cargo.toml
    echo "tools/card/bump.sh: the edit did not take (read back '$CHECK'); Cargo.toml restored" >&2
    exit 1
fi

# Cargo.lock carries every workspace member's version too, and it is checked in.
# Left alone it goes stale silently — which is the state this repository was
# actually in when ADR-027 §10 was written: Cargo.toml said 0.1.0 and the lock
# still said 0.0.1.
cargo metadata --format-version 1 >/dev/null

# --- and so do the nested workspaces under examples/ -------------------------
#
# **This is the second half of the same fact, and leaving it out broke the
# sequence twice on 2026-09-04.** Every application under `examples/` is its own
# workspace (its Cargo.toml carries a bare `[workspace]`, because `cargo rivet
# package` builds it for a board and the root workspace is the operating
# system's). Each therefore has its OWN Cargo.lock, tracked, and each names the
# root's path packages -- `rivetos-sdk`, `aura-card` -- BY VERSION.
#
# Those locks are not touched by `cargo metadata` above, which only knows the
# root workspace. They pick the new number up on the next build of that example
# -- which happens AFTER this script's commit. So the tree is dirty by the time
# anybody looks, and ADR-027 §10.2's refusal fires on the NEXT bump, blaming a
# tree that nobody edited.
#
# **Rewritten rather than regenerated, deliberately.** `cargo metadata` in each
# example would resolve its whole graph: it needs a network or a warm registry
# cache, it needs the cross-compilation config some of them carry, and it can
# move an unrelated dependency in a commit that ADR-027 §10.2 requires to
# contain the version and nothing else. Editing the one field is offline,
# deterministic, and cannot change what an example resolves to.
#
# The rewrite is UNCONDITIONAL on the old value, not old->new, because a lock
# whose example has not been built for two bumps is stale by more than one
# number and would otherwise be left stale.
#
# The rewrite goes through a scratch directory OUTSIDE the tree: a temporary
# file beside the lock would itself be an untracked file, and if anything
# failed part-way it would be left behind to make the next bump refuse -- the
# very failure this section exists to remove.
BUMP_TMP=$(mktemp -d)
trap 'rm -rf "$BUMP_TMP"' EXIT

for lock in $NESTED_LOCKS; do
    # **The working copy is CRLF and awk cannot see it.** Git Bash's awk and
    # sed are MSYS builds: they open files in TEXT mode, so `$0` never carries
    # the CR and anything they write comes out LF. Left there, the rewrite
    # would silently convert eleven lockfiles from CRLF to LF -- which does not
    # dirty the tree (Windows git's clean filter normalises either form to LF,
    # AGENTS.md §10.5) but turns a one-line change into a whole-file one in
    # every reviewer's diff tool, and makes `git add` warn on all of them.
    #
    # So the ending is measured with `tr`, which is byte-oriented, and put back
    # afterwards. `head -c` keeps that off the whole file.
    CRLF=
    if head -c 4096 "$lock" | tr -cd '\r' | grep -q .; then
        CRLF=1
    fi

    awk -v names="$WORKSPACE_PKGS" -v new="$NEXT" '
        BEGIN { n = split(names, a, " ")
                for (i = 1; i <= n; i++) if (a[i] != "") want[a[i]] = 1 }
        # A [[package]] block is buffered whole, because `source` appears
        # AFTER `version` and a registry package that happens to share a
        # member name must not be rewritten.
        function flush(   i, t) {
            for (i = 0; i < nb; i++) {
                t = buf[i]
                if (!src && (nm in want) && t ~ /^version = /)
                    t = "version = \"" new "\""
                print t
            }
            nb = 0; nm = ""; src = 0; inblk = 0
        }
        $0 == "[[package]]" {
            if (inblk) flush()
            inblk = 1; buf[nb++] = $0
            next
        }
        {
            if (!inblk) { print; next }
            buf[nb++] = $0
            if ($0 ~ /^name = /)   { nm = $0
                                     sub(/^name = "/, "", nm); sub(/"$/, "", nm) }
            if ($0 ~ /^source = /) src = 1
            if ($0 == "") flush()
        }
        END { if (inblk) flush() }
    ' "$lock" > "$BUMP_TMP/lock"

    if [ -n "$CRLF" ]; then
        sed 's/$/\r/' "$BUMP_TMP/lock" > "$BUMP_TMP/lock.crlf"
        mv -f "$BUMP_TMP/lock.crlf" "$BUMP_TMP/lock"
    fi

    # A `dependencies` entry names a package by name ALONE unless two versions
    # of that name are in the graph, in which case it is "name version" and the
    # block rewrite above is not enough. That is not the case today; refuse
    # rather than half-rewrite the day it becomes so.
    for pkg in $WORKSPACE_PKGS; do
        if grep -q "^ \"$pkg [0-9]" "$BUMP_TMP/lock"; then
            echo "tools/card/bump.sh: $lock names '$pkg' with a version in a" >&2
            echo "  dependencies list, which means the graph now holds two of it." >&2
            echo "  Rewriting only the [[package]] block would leave the lock" >&2
            echo "  inconsistent. Regenerate this lock by hand and commit it." >&2
            exit 1
        fi
    done

    cp "$BUMP_TMP/lock" "$lock"
done

# --- commit that one change alone (step 3) -----------------------------------
git add Cargo.toml Cargo.lock
if [ -n "$NESTED_LOCKS" ]; then
    git add $NESTED_LOCKS
fi
git commit --quiet -m "Version $NEXT" -m "ADR-027 §10.2: the bump is committed before the build, so version and
commit are a 1:1 mapping and no image is ever stamped -dirty for a number
that was minted and not recorded.

The nested workspaces under examples/ carry their own lockfiles naming this
workspace's path packages by version, so they are refreshed in this same
commit. Left out, they are rewritten by the next build of an example -- after
this commit -- and the tree is dirty for a change nobody made, which makes the
NEXT bump refuse."

# --- the sequence must END clean, or the next bump refuses (step 4) ----------
#
# The refusal at the top of this script is only as good as this check: if the
# bump leaves anything behind, the next run blames a tree nobody edited. Say so
# HERE, where the cause is one screen away, rather than a day later.
LEFT=$(git status --porcelain)
if [ -n "$LEFT" ]; then
    echo "tools/card/bump.sh: the version was committed but the tree is NOT clean." >&2
    echo "  Something carries the workspace version and was not refreshed above." >&2
    echo "$LEFT" | sed 's/^/    /' >&2
    exit 1
fi

git describe --tags --always --dirty
