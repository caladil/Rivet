#!/bin/sh
# Lint and test EVERY Rust workspace in this repository — not just the root one.
#
#   tools/container/exec.sh tools/test-all.sh
#   tools/container/exec.sh tools/test-all.sh aura        # only matching workspaces
#   tools/container/exec.sh tools/test-all.sh '' --bless  # rewrite the lint baseline
#
# ## Why this exists
#
# `cargo test --workspace` at the root tests the operating system. It does not
# test `examples/aura/aura-renderer`, or `examples/templates/template-kiosk`, or
# any of the other trees under `examples/`, `deploy/` and `tools/`, because each
# of those declares its own `[workspace]` — deliberately, and each says why in
# its own `Cargo.toml`: `cargo rivet package` builds them for a board and the
# root workspace is the operating system's.
#
# That is a good reason with a cost nobody had paid attention to. The audit of
# 2026-09-03 (`PLAN.md` §123) found a test in `aura-renderer` that **cannot
# pass** — it asserts `Command::Unhandled("rotate_corner")` against a function
# that returns `Command::RotateCorner`, and `Command` derives `PartialEq`. It had
# been committed and left, because no command in this repository had ever
# compiled that crate's tests.
#
# It was found by a person reading. This is so the next one is found by a machine
# running.
#
# ## The three checks, and why only one of them is a gate today
#
# **Tests are a gate.** A failing test fails this script.
#
# **Lints are a RATCHET, not a gate.** `cargo clippy -- -D warnings` does not
# pass on this tree: the root workspace answered 126 warnings and 0 errors on
# 2026-09-03. Turning that into a gate would mean either a large unrelated
# cleanup landing inside a security remediation, or `#[allow]`s scattered to buy
# a green run — and the second is how a lint gate stops meaning anything. So the
# count is compared against `tools/lint-baseline` and this script fails when it
# GOES UP. The number can only come down, nobody has to clean 126 warnings
# today, and a new one cannot arrive unnoticed.
#
# **Formatting is REPORTED and is not a gate at all.** `cargo fmt --check`
# produces 1718 hunks against this tree with default settings and 4019 with
# `use_small_heuristics = "Max"`, measured 2026-09-03. This tree has never been
# rustfmt-clean; adopting it means a one-time reformat of the whole operating
# system, which rewrites `git blame` for every line and is a maintainer's call
# and not a script's. Until that is ruled, the drift is printed as a number.
#
# **Rustdoc warnings are a second RATCHET, the same shape as lints.** `cargo
# doc --workspace --no-deps` had apparently never been run against this tree
# before `.rulings/` R-153's Phase 2d — the count is pinned in
# `tools/source/rustdoc-baseline.txt` and this script fails when it goes UP,
# for the same reason clippy's baseline is a ratchet and not a gate: a
# pre-existing backlog nobody has cleared is not the same defect as a NEW
# warning arriving unnoticed.
#
# ## What it refuses to do
#
# **Skip silently.** A workspace with no Rust source of its own — the Go
# controller, the C++ renderer, the C greeter probe, each of which carries a
# `Cargo.toml` purely to hold a `[package.metadata.rivet]` manifest — is listed
# in the summary by name and with the reason. A tool missing from the toolchain
# is reported as a named gap and not as a pass. And if discovery finds no nested
# workspace at all, this exits non-zero: there are more than a dozen, so finding
# none means the walk broke, and reporting "all clear" would be exactly the
# failure this script was written to prevent.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
cd "$REPO"

FILTER=${1:-}
BLESS=no
[ "${2:-}" = "--bless" ] && BLESS=yes

BASELINE="$HERE/lint-baseline"
FRESH=$(mktemp)
trap 'rm -f "$FRESH"' EXIT INT TERM

# --- what the toolchain can actually do -------------------------------------
#
# Checked once, up front, and reported. Version 3 of the build image printed
# nothing from `cargo clippy` and exited non-zero, so a lint run read as clean
# while 287 warnings stood (`tools/container/image-version.md`). The lesson is
# that a missing tool must be LOUD, so each of these becomes a named line in the
# summary rather than a check that quietly does not happen.
HAVE_FMT=no
HAVE_CLIPPY=no
HAVE_DENY=no
HAVE_UV=no
cargo fmt --version >/dev/null 2>&1 && HAVE_FMT=yes
cargo clippy --version >/dev/null 2>&1 && HAVE_CLIPPY=yes
cargo deny --version >/dev/null 2>&1 && HAVE_DENY=yes
uv --version >/dev/null 2>&1 && HAVE_UV=yes

# --- discovery ---------------------------------------------------------------
#
# `find` rather than `git ls-files`, because this runs inside a container where
# the repository is a bind mount and git refuses it as "dubious ownership". The
# exclusions are by path prefix: build output, the agent worktrees, and every
# dot-directory (which is where scratch packages written by hand during a board
# session end up — `.launcher-pkg`, `.qemu-endpoints`).
#
# **`vendor/` WAS excluded and no longer is** — 2026-09-04, when the maintainer
# ruled `vendor-and-own` for `ixr-smb-server` (`.rulings/` R-134). The exclusion
# was written when `vendor/` held firmware blobs and nothing else, and it was
# right then. It is exactly wrong now: **owning a fork means running its suite**,
# and a runner that skipped the one crate this project has adopted from outside
# would be skipping the code with the worst track record in the tree — four
# defects found by its first real consumer. Its 150 tests now run on every pass.
#
# The lints are still the crate's own business: `Cargo.toml`'s `exclude` keeps
# it out of the root workspace, so `[workspace.lints]` does not reach it and its
# count is baselined separately like any other nested workspace.
#
# `cmd/cargo-rivet/tests/fixtures/` is excluded on purpose and not as tidying:
# those trees are INPUT to the packaging tool's own tests, which build them
# already. Linting a fixture would be linting a test's data.
workspaces() {
    find . -name Cargo.toml -type f \
        -not -path './target/*' \
        -not -path './.git/*' \
        -not -path './.claude/*' \
        -not -path './.*/*' \
        -not -path './cmd/cargo-rivet/tests/fixtures/*' \
        -print \
    | while read -r manifest; do
        grep -q '^\[workspace\]' "$manifest" || continue
        dir=$(dirname "$manifest")
        printf '%s\n' "${dir#./}"
    done | sort
}

ALL=$(workspaces)
NESTED=$(printf '%s\n' "$ALL" | grep -v '^\.$' || true)

if [ -z "$NESTED" ]; then
    echo "tools/test-all.sh: discovery found NO nested workspace." >&2
    echo "    There are more than a dozen under examples/, deploy/ and tools/." >&2
    echo "    Finding none means this walk is broken, and reporting a pass would" >&2
    echo "    be the exact failure this script exists to prevent. Refusing." >&2
    exit 1
fi

# --- the run -----------------------------------------------------------------

PASSED=""
FAILED=""
SKIPPED=""
NOTES=""
FMT_DRIFT=0

note() { NOTES="$NOTES
$1"; }

# Every `.rs` file this workspace owns — its own tree, minus anything inside a
# NESTED workspace. Without the subtraction the root would claim every example's
# sources as its own, and the total would count them twice.
own_sources() {
    _mine=$1
    if [ "$_mine" = "." ]; then _root=.; else _root="./$_mine"; fi
    find "$_root" -name '*.rs' -type f \
        -not -path './target/*' \
        -not -path './.git/*' \
        -not -path './.claude/*' \
        -not -path './.*/*' \
        -not -path './cmd/cargo-rivet/tests/fixtures/*' \
        -print 2>/dev/null \
    | while read -r _f; do
        _skip=no
        for _other in $NESTED; do
            [ "$_other" = "$_mine" ] && continue
            case "$_f" in "./$_other/"*) _skip=yes; break ;; esac
        done
        if [ "$_skip" = no ]; then printf '%s\n' "$_f"; fi
    done
}

baseline_for() {
    # The recorded count for one workspace, or `-` when it has none yet.
    [ -f "$BASELINE" ] || { printf -- '-'; return; }
    awk -v w="$1" '$1 == w { print $2; found = 1 } END { if (!found) print "-" }' "$BASELINE"
}

check_one() {
    dir=$1
    label=$dir
    [ "$label" = "." ] && label="(root)"

    if [ -n "$FILTER" ]; then
        case "$dir" in
            *"$FILTER"*) ;;
            *) return 0 ;;
        esac
    fi

    # A workspace with no Rust source of its own. The Go controller, the C++
    # renderer and the C greeter each carry a Cargo.toml to hold a
    # `[package.metadata.rivet]` manifest and nothing else, so there is nothing
    # here to lint or test. Named, so the skip is visible.
    if [ "$dir" != "." ] \
       && [ ! -f "$dir/src/main.rs" ] && [ ! -f "$dir/src/lib.rs" ]; then
        SKIPPED="$SKIPPED $label"
        note "  SKIPPED  $label — a manifest with no Rust source (built by its own recipe)"
        return 0
    fi

    # A workspace that has DECLARED it cannot be built here, and why.
    #
    # A marker file rather than a list in this script: the reason belongs beside
    # the thing it is about, where whoever changes that thing will see it. The
    # file's first line is printed, so a skip is never just an absence — and a
    # workspace with no marker that fails to build is a FAILURE, which is what
    # keeps this from becoming a way to hide one.
    if [ -f "$dir/.no-host-test" ]; then
        SKIPPED="$SKIPPED $label"
        note "  SKIPPED  $label — $(head -1 "$dir/.no-host-test")"
        return 0
    fi

    # A target directory per workspace. They would otherwise share one, and two
    # workspaces resolving the same crate to different features rebuild each
    # other's dependencies on every alternation — minutes, silently, forever.
    slug=$(printf '%s' "$label" | tr -c 'A-Za-z0-9' '-')
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        TARGET="$CARGO_TARGET_DIR/test-all/$slug"
    else
        TARGET="$REPO/target/test-all/$slug"
    fi

    echo ""
    echo "=============================================================="
    echo "  $label"
    echo "=============================================================="

    ok=yes
    run() { ( cd "$dir" && CARGO_TARGET_DIR="$TARGET" "$@" ); }

    # --- formatting: counted, never fatal. See the header.
    #
    # `rustfmt` over this workspace's OWN files rather than `cargo fmt --all`.
    # `cargo fmt --all` in a nested workspace formats its path dependencies too,
    # so every crate that depends on `rivet-sdk` reported the root tree's ~1718
    # hunks as its own and the total counted them a dozen times over.
    if [ "$HAVE_FMT" = yes ]; then
        files=$(own_sources "$dir")
        if [ -n "$files" ]; then
            # shellcheck disable=SC2086
            drift=$(rustfmt --edition 2024 --check $files 2>/dev/null | grep -c '^Diff in' || true)
            FMT_DRIFT=$((FMT_DRIFT + drift))
            [ "$drift" -eq 0 ] \
                || note "  format   $label — $drift hunk(s) of drift (reported, not a gate)"
        fi
    fi

    # --- lints: a ratchet against tools/lint-baseline.
    if [ "$HAVE_CLIPPY" = yes ]; then
        out=$(run cargo clippy --all-targets --all-features --message-format short 2>&1) || {
            ok=no
            note "  FAIL     $label — clippy could not compile it"
            printf '%s\n' "$out" | grep -E '^error' | head -20
        }
        count=$(printf '%s\n' "$out" | grep -cE ': warning: ' || true)
        printf '%s %s\n' "$label" "$count" >>"$FRESH"
        was=$(baseline_for "$label")
        if [ "$was" = "-" ]; then
            note "  lints    $label — $count warning(s), NO BASELINE (run with --bless)"
        elif [ "$count" -gt "$was" ]; then
            ok=no
            note "  FAIL     $label — clippy warnings went UP: $was -> $count"
            printf '%s\n' "$out" | grep -E ': warning: ' | head -20
        elif [ "$count" -lt "$was" ]; then
            note "  lints    $label — $count warning(s), DOWN from $was. Bless it to lock the gain in"
        else
            note "  lints    $label — $count warning(s), level with the baseline"
        fi
    fi

    # --- tests: the gate.
    run cargo test --all-targets || { ok=no; note "  FAIL     $label — tests"; }

    # `--all-targets` does not run doctests, and the root workspace has them.
    #
    # A crate with no library target answers *"no library targets found in
    # package"* and exits non-zero, which is not a failure — it is most of
    # `examples/`, every one of which is a single binary. Distinguished by the
    # message rather than by guessing from the file layout, because a workspace
    # can hold a lib its root directory does not show.
    if doc=$(run cargo test --doc 2>&1); then
        :
    elif printf '%s' "$doc" | grep -q 'no library targets found'; then
        :
    else
        ok=no
        note "  FAIL     $label — doctests"
        printf '%s\n' "$doc" | tail -20
    fi

    if [ "$ok" = yes ]; then
        PASSED="$PASSED $label"
    else
        FAILED="$FAILED $label"
    fi
}

for w in $ALL; do
    check_one "$w"
done

# --- optional features nothing in the tree turns on --------------------------
#
# `cargo test --all-targets` above resolves the root workspace with every
# optional feature OFF, which is correct: `rivetos-sdk`'s `draw` and `serve` are
# opt-in and no crate in this workspace asks for either. The cost is that the
# tests behind them are **compiled by nothing**, which is the same failure this
# whole script was written for — `aura-renderer` had a test that could not pass
# because no command compiled that crate.
#
# It had already been noticed once and not closed. `screen` was split out of
# `draw` on 2026-09-01 precisely so that drawing could be TESTED on a host with
# no `libdrm`, and the manifest note says that buys
# `cargo test -p rivetos-sdk --features draw` "on any machine" — a command
# nothing ran. This is that command, run.
#
# **`screen` is deliberately not in the list.** It links `libdrm`, which lives
# only in the aarch64 sysroot, so a test binary enabling it does not link here.
# That is why `flip.rs` exists: the parts of the display with judgement in them
# are in a module that compiles without the C.
OPTIONAL=skipped
if [ -z "$FILTER" ]; then
    echo ""
    echo "=============================================================="
    echo "  (root) — rivetos-sdk, optional features"
    echo "=============================================================="
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        FEATURED="$CARGO_TARGET_DIR/test-all/-root--features"
    else
        FEATURED="$REPO/target/test-all/-root--features"
    fi
    # `composited-gpu` (which implies `composited`) is in the list because
    # the compositor client's connection handling is tested in those modules
    # and only examples outside this workspace turn the feature on. It links
    # the host's GBM and EGL, which this container carries.
    if CARGO_TARGET_DIR="$FEATURED" cargo test -p rivetos-sdk --features draw,serve,composited-gpu --all-targets \
        && CARGO_TARGET_DIR="$FEATURED" cargo test -p rivetos-sdk --features draw,serve,composited-gpu --doc; then
        OPTIONAL=pass
    else
        OPTIONAL=FAIL
        note "  FAIL     (root) — rivetos-sdk with --features draw,serve,composited-gpu"
    fi
fi

# --- the dependency check ----------------------------------------------------
#
# Root workspace only: `deny.toml` is the operating system's policy and the
# example trees are not shipped on a card.
# **`licenses bans sources` gate; `advisories` is REPORTED — split 2026-09-06,
# `PLAN.md` §155, the day this was first run against the tree.**
#
# The three that gate are properties of what is IN the tree: a licence, a
# duplicate, a registry. They change when somebody changes a dependency, which
# is exactly when a gate should fire.
#
# **`advisories` is different in kind and gating it would get it turned off.**
# It compares against a database fetched from the network and it moves on
# somebody else's clock: a crate marked `unmaintained` overnight turns a build
# red with no change to this repository, and the first response to that is
# always to disable the check rather than to act on it. Today it reports two —
# `rustls-pemfile` and `ttf-parser`, both unmaintained, neither a
# vulnerability — with **zero vulnerabilities and zero yanked crates**.
#
# It is not being ignored: it is printed every run, and the day it reports a
# VULNERABILITY that is a different sentence and wants a different response.
DENY=skipped
DENY_ADVISORIES=skipped
if [ -z "$FILTER" ] && [ "$HAVE_DENY" = yes ]; then
    if cargo deny check licenses bans sources; then DENY=pass; else DENY=FAIL; fi
    if cargo deny check advisories; then
        DENY_ADVISORIES=clean
    else
        DENY_ADVISORIES="findings — see the output above"
    fi
fi

# --- the record's own claims -------------------------------------------------
#
# **Three of `claims.py`'s four checks, and deliberately not the fourth.**
#
# `paths`, `counts` and `crates` all reached **zero** on 2026-09-06
# (`PLAN.md` §154), and a check at zero is the only kind that can be a gate: one
# that reports findings on every run reports nothing, because the new finding —
# the one that means a mistake was just made — is invisible in the pile.
#
# **`citations` is excluded, and that is not squeamishness.** It stands at 1,684
# and is `Q9`'s open ruling; including it would make this gate fail on every run
# from the day it was added, which is the failure mode this whole block exists
# to avoid. It goes in the day that number is dealt with.
#
# Root workspace only, like the dependency check above: these are claims about
# the operating system's own record.
CLAIMS=skipped
if [ -z "$FILTER" ] && [ "$HAVE_UV" = yes ]; then
    if uv run tools/docs/claims.py --only paths,counts,crates; then
        CLAIMS=pass
    else
        CLAIMS=FAIL
    fi
fi

# --- the command reference ---------------------------------------------------
#
# **A GATE, unconditionally, and that is the opposite of the block above.**
# `docs/COMMANDS.md` is generated from every binary's own `--help`
# (`tools/docs/commands.py`, plan `PLAN-Feature-Set-2026-09-12.md` §9.1) and
# `CLAIMS` above is skipped in exactly the environment this script usually
# runs in, because the build container has neither `uv` nor a Python
# interpreter — checked 2026-09-13. A doc check that only runs on a host
# nobody routes through this script would gate nothing, so this one does NOT
# reuse that generator: `tools/docs/check-commands.sh` is POSIX `sh`, builds
# the workspace's binaries (already needed for everything else here) and
# diffs each one's live `--help` against what is committed.
#
# Root workspace only — the binaries it checks all live in `cmd/`.
COMMANDS=skipped
if [ -z "$FILTER" ]; then
    if sh "$HERE/docs/check-commands.sh"; then
        COMMANDS=pass
    else
        COMMANDS=FAIL
    fi
fi

# --- rustdoc warnings: a ratchet against tools/source/rustdoc-baseline.txt ---
#
# Added for `.rulings/` R-153's Phase 2d. `cargo doc --workspace --no-deps`
# had apparently never been run against this tree before it — `STATE.md`'s own
# note going in was that the core crate alone carried dozens of warnings. That
# is the same shape as the clippy ratchet above and gets the same mechanism
# rather than a new one — a rising count fails, a falling one prints FELL, and
# `--bless` rewrites the pin.
#
# **This is root-workspace only and Linux-only in practice.** `cargo doc`
# compiles every crate it documents, and two of this workspace's are Linux-only
# (`AGENTS.md` §10.5) — the same reason `cargo test --workspace` fails on bare
# Windows and it is "not a real failure". Nothing here detects the host and
# prints NOT CHECKED for it, because `cargo test` above does not either: this
# script's own header says to run it in the container, and a Windows run
# failing here fails the same way and for the same reason.
RUSTDOC_BASELINE="$HERE/source/rustdoc-baseline.txt"
RUSTDOC=skipped
if [ -z "$FILTER" ]; then
    echo ""
    echo "=============================================================="
    echo "  (root) — cargo doc --workspace --no-deps"
    echo "=============================================================="
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        DOCTARGET="$CARGO_TARGET_DIR/test-all/-root--doc"
    else
        DOCTARGET="$REPO/target/test-all/-root--doc"
    fi
    doc_rc=0
    doc_out=$(CARGO_TARGET_DIR="$DOCTARGET" cargo doc --workspace --no-deps 2>&1) || doc_rc=$?
    printf '%s\n' "$doc_out"
    if [ "$doc_rc" -ne 0 ]; then
        # `cargo doc` exits 0 on warnings alone; a non-zero exit is a real
        # compile error, not a bigger pile of warnings, and counting `warning:`
        # lines from a build that never finished would misreport a failure as
        # some smaller number of them.
        RUSTDOC=FAIL
        note "  FAIL     (root) — cargo doc exited $doc_rc, not just warnings"
    else
        # `warning: ... generated N warnings` is rustdoc's own SUMMARY line for
        # a crate, restated at the end — it starts with `warning:` like every
        # line it is summarizing, and counting it would count each real
        # warning twice.
        doc_count=$(
            printf '%s\n' "$doc_out" \
            | grep -E '^warning:' \
            | grep -vE 'generated [0-9]+ warnings?' \
            | wc -l | tr -d ' '
        )
        if [ -f "$RUSTDOC_BASELINE" ]; then
            rustdoc_was=$(grep -vE '^[[:space:]]*(#|$)' "$RUSTDOC_BASELINE" | head -1)
        else
            rustdoc_was=""
        fi
        if [ -z "$rustdoc_was" ]; then
            note "  rustdoc  (root) — $doc_count warning(s), NO BASELINE (run with --bless)"
            RUSTDOC="no baseline"
        elif [ "$doc_count" -gt "$rustdoc_was" ]; then
            RUSTDOC=FAIL
            note "  FAIL     (root) — rustdoc warnings went UP: $rustdoc_was -> $doc_count"
        elif [ "$doc_count" -lt "$rustdoc_was" ]; then
            RUSTDOC=pass
            note "  rustdoc  (root) — $doc_count warning(s), DOWN from $rustdoc_was. Bless it to lock the gain in"
            echo ""
            echo "FELL $rustdoc_was -> $doc_count rustdoc warning(s)"
        else
            RUSTDOC=pass
            note "  rustdoc  (root) — $doc_count warning(s), level with the baseline"
        fi
    fi
fi

# --- the provisioning ceremony, both adapters --------------------------------
#
# **A GATE, not a ratchet**, because both harnesses are at zero findings and a
# check at zero is the only kind that can gate: one that reports on every run
# reports nothing.
#
# `rehearse.sh` drives the command line; `rehearse-menu.sh` drives the MENU
# under a pty and holds the certificate it mints beside the other's. The second
# is why both are here rather than one: `ADR-006` §3.1's co-equality is not
# satisfied by both interfaces merely HAVING an operation, and these two once
# composed different certificates from the same request, each internally
# consistent. **Only a check spanning both catches that**, and no unit test can,
# because each adapter passes its own.
#
# They cost two `cargo build --release` invocations and about a minute, which is
# the maintainer's ruling of 2026-09-07 rather than this script's judgement.
#
# `rehearse-member.sh` is the third: a fresh fleet server and then a member of
# it, each through `cargo rivet provision`'s own interview under a pty, in one
# scratch `--at`. It is here because a member staged without its server's
# `rivet-fleetd` key boots, is listed, and cannot be managed, and no unit test
# spans the two interviews that decide it. It adds a release build of
# `cargo-rivet`.
#
# Root workspace only, and named in the summary when skipped rather than
# vanishing: `script(1)` is the pty and `openssl` reads the certificates back,
# and a run without either proves less than it appears to.
CEREMONY=skipped
if [ -z "$FILTER" ]; then
    if ! command -v script >/dev/null 2>&1; then
        CEREMONY="no script(1)"
    elif ! command -v openssl >/dev/null 2>&1; then
        CEREMONY="no openssl"
    else
        CEREMONY=pass
        for harness in tools/provision/rehearse.sh tools/provision/rehearse-menu.sh \
            tools/provision/rehearse-member.sh; do
            if sh "$harness" >"$FRESH.ceremony" 2>&1; then
                note "  ceremony (root) — $(basename "$harness") passed"
            else
                CEREMONY=FAIL
                note "  ceremony (root) — $(basename "$harness") FAILED"
                cat "$FRESH.ceremony" >&2
            fi
        done
        rm -f "$FRESH.ceremony"
    fi
fi

# --- the baseline ------------------------------------------------------------

if [ "$BLESS" = yes ]; then
    if [ -n "$FILTER" ]; then
        echo "tools/test-all.sh: refusing to bless a FILTERED run — it would drop" >&2
        echo "    every workspace the filter excluded from the baseline." >&2
        exit 1
    fi
    sort "$FRESH" >"$BASELINE"
    echo ""
    echo "tools/test-all.sh: wrote $BASELINE"
    if [ -z "$FILTER" ]; then
        {
            echo "# Warnings from \`cargo doc --workspace --no-deps\`, root workspace only."
            echo "# A rise fails this script; a fall prints FELL and is not a failure."
            echo "# Regenerate: tools/container/exec.sh tools/test-all.sh '' --bless"
            echo "$doc_count"
        } >"$RUSTDOC_BASELINE"
        echo "tools/test-all.sh: wrote $RUSTDOC_BASELINE"
    fi
fi

# --- the summary -------------------------------------------------------------

echo ""
echo "=============================================================="
echo "  tools/test-all.sh"
echo "=============================================================="
if [ -n "$NOTES" ]; then printf '%s\n' "$NOTES"; fi
echo ""
echo "  passed:  ${PASSED:-none}"
echo "  failed:  ${FAILED:-none}"
echo "  skipped: ${SKIPPED:-none}"
echo ""

# The gaps, stated rather than left to be inferred from an absence of output.
if [ "$HAVE_FMT" = yes ]; then
    echo "  formatting drift: $FMT_DRIFT hunk(s) — REPORTED, NOT A GATE."
    echo "    This tree has never been rustfmt-clean. Adopting rustfmt means a"
    echo "    one-time reformat of the whole operating system, which is a"
    echo "    maintainer's call. Until then this is a number, not a verdict."
else
    echo "  NOT CHECKED: formatting — no cargo-fmt in this toolchain"
fi
[ "$HAVE_CLIPPY" = yes ] || echo "  NOT CHECKED: lints — no cargo-clippy in this toolchain"
[ "$OPTIONAL" = skipped ] || echo "  rivetos-sdk --features draw,serve,composited-gpu: $OPTIONAL"
if [ "$HAVE_DENY" = yes ]; then
    [ "$DENY" = skipped ] || echo "  cargo deny (licenses, bans, sources): $DENY"
    [ "$DENY_ADVISORIES" = skipped ] || \
        echo "    advisories: $DENY_ADVISORIES — REPORTED, NOT A GATE (PLAN.md §155)"
else
    echo "  NOT CHECKED HERE: dependencies — no cargo-deny in this toolchain."
    echo "    It DOES run, on the host, and passed on 2026-09-06: licenses ok,"
    echo "    bans ok, sources ok, no GPL/LGPL/AGPL anywhere (PLAN.md §155)."
    echo "    Installing it in the container is one Dockerfile line."
fi
if [ "$HAVE_UV" = yes ]; then
    [ "$CLAIMS" = skipped ] || echo "  the record's claims (paths, counts, crates): $CLAIMS"
    echo "    NOT GATED: citations — 1,684 findings, PLAN.md §154 and Q9."
else
    echo "  NOT CHECKED HERE: the record's claims — no uv and no Python here."
    echo "    They DO run, on the host, and pass: paths 0, counts 0, crates 0."
fi
[ "$COMMANDS" = skipped ] || echo "  the command reference (docs/COMMANDS.md vs live --help): $COMMANDS"
[ "$RUSTDOC" = skipped ] || echo "  rustdoc warnings (root, --no-deps): $RUSTDOC"
case "$CEREMONY" in
    skipped) ;;
    pass|FAIL) echo "  the provisioning ceremony (command line, menu, server then member): $CEREMONY" ;;
    *)  echo "  NOT CHECKED: the provisioning ceremony — $CEREMONY."
        echo "    The menu is driven under a pty and its certificate compared"
        echo "    with \`ca server\`'s; neither is possible without it." ;;
esac
echo ""

# --- the verdict --------------------------------------------------------------
#
# **One variable, computed once, and everything downstream reads it** —
# `PLAN.md` §137.7: a run whose root workspace could not compile was reported,
# by this same script, as `(root) — clippy could not compile it` and
# `(root) — tests`, and was read afterward as a pass. Direct reproduction
# here — the same struct literal broken and restored twice, in the container,
# with the exit code checked in the SAME command that ran the script — could
# not make this script hand back 0 while `$FAILED` was non-empty: the three
# checks that used to sit here have set the process exit status correctly
# every time it was tried. So the likelier failure is downstream of this
# file: `$?` read in a later shell (this project's own tooling documents that
# shell state does not survive between commands), or a long run's tail —
# where a verdict like this one lives — never reached because the middle of
# the output was the only part still on screen.
#
# This line exists for that reader. It is the LAST thing this script prints,
# it names its own exit code so the two cannot silently disagree, and it says
# `FAIL` in a way a truncated read of everything BEFORE it cannot turn into a
# pass. It changes nothing about what makes the run fail — `run cargo test`
# above, checked with `||`, is what actually holds the door; this is only the
# sign on it.
if [ -n "$FAILED" ] || [ "$OPTIONAL" = FAIL ] || [ "$DENY" = FAIL ] || [ "$CLAIMS" = FAIL ] \
   || [ "$COMMANDS" = FAIL ] || [ "$RUSTDOC" = FAIL ] || [ "$CEREMONY" = FAIL ]; then
    VERDICT=1
    echo "  RESULT: FAIL — exiting 1. Do not read anything above this line as a pass."
else
    VERDICT=0
    echo "  RESULT: PASS — exiting 0."
fi
echo ""

exit "$VERDICT"
