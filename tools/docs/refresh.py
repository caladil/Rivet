# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Regenerate the generated documents and re-run every doc guard, in one command.

Added 2026-09-07 for `.rulings/` R-153's Phase 2c. Before this, "regenerate the
index, check the claims, check the rulings, check the prose" was four separate
commands a person or a hook had to remember and run in order — and the claims
baseline in particular has a direction that matters (see below), which a
person re-running `--write-baseline` by habit can get backwards.

    uv run tools/docs/refresh.py            # regenerate and check; do not stage
    uv run tools/docs/refresh.py --stage    # also `git add` what it regenerated

## The one rule that must not be got wrong

**The `claims.py` baseline ratchets DOWN automatically and NEVER up.**
`plan-citations`, `rot-markers`, `citations` and `deep-citations` are ratchets
(`RATCHET_ONLY` in `claims.py`): a falling count is good news and this script
re-baselines to lock it in, but a RISING count is a regression and must fail
loudly with the baseline left exactly as it was. Regenerating the baseline on
a rise would silence the gate that a rise is supposed to trip — the baseline
would simply absorb the new, worse number and `--check` would read clean on
the very commit that made things worse.

So the sequence below asks `claims.py --check` for its verdict FIRST, and only
calls `--write-baseline` when that verdict was already a pass. A `--check`
that exits 1 (something NEW, SPREAD, or a pinned finding that stopped firing)
stops this script here, with the findings printed and the baseline file
untouched — bit for bit, which the acceptance test for this file proves with
a `sha256sum` before and after rather than trusting the description.

## What runs, in order, and why this order

1. `index.py` regenerates `docs/INDEX.md` and `docs/index.json`
   unconditionally — the claims checks that follow read the corpus, not the
   index, but a stale index is its own defect and there is no reason to wait
   to find it.
2. `claims.py --check` — the ratchet logic above.
3. `rulings.py` — its exact half is a gate; a quotation of the maintainer that
   reached `docs/` fails the run.
4. `prose.py --check` — a run of six or more spaces in a string literal, the
   signature of a lost line-continuation backslash.

A tool earlier in this list failing stops the ones after it — there is no
value in reporting a stale index against documents the next guard is about to
say are wrong for an unrelated reason, and it keeps the "what broke" question
answerable from the first failure rather than a wall of them.

## What `--stage` does, and does not

`git add`s exactly three paths: `docs/INDEX.md`, `docs/index.json` and
`tools/docs/claims-baseline.txt` — the three files this script itself can
rewrite. Nothing else. A commit hook that staged everything would turn "add
one file" into "commit whatever happens to be sitting in the tree", which is
not this tool's business to decide.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

# See claims.py's own note: a Windows console defaults to cp1252 and mangles
# every `§` and em dash this corpus is full of.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

ROOT = Path(__file__).resolve().parents[2]
DOCS_TOOLS = Path(__file__).resolve().parent
SOURCE_TOOLS = ROOT / "tools" / "source"

INDEX_PY = DOCS_TOOLS / "index.py"
CLAIMS_PY = DOCS_TOOLS / "claims.py"
RULINGS_PY = DOCS_TOOLS / "rulings.py"
PROSE_PY = SOURCE_TOOLS / "prose.py"

# The only three paths `--stage` ever touches — see the module docstring.
STAGED_PATHS = ("docs/INDEX.md", "docs/index.json", "tools/docs/claims-baseline.txt")


def run(script: Path, *args: str) -> subprocess.CompletedProcess[str]:
    """Run a sibling tool with THIS interpreter — the one `uv run` already
    resolved to launch this script — rather than shelling out to a bare
    `python` that is not on this maintainer's PATH at all (`AGENTS.md` §1).

    **`PYTHONIOENCODING=utf-8` is forced on the child.** `claims.py` and
    `index.py` reconfigure their own streams to UTF-8 because a Windows
    console otherwise defaults to cp1252 and mangles every `§` and em dash
    this corpus is full of — but `rulings.py` and `prose.py` do not, and a
    piped (non-console) stdout is not a console either, so a child can end up
    encoding its own `—` as cp1252 while this parent decodes strictly as
    UTF-8. Setting it here is one fix for every child rather than four.
    """
    env = dict(os.environ, PYTHONIOENCODING="utf-8")
    return subprocess.run(
        [sys.executable, str(script), *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=env,
    )


def echo(result: subprocess.CompletedProcess[str]) -> None:
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)


def main() -> int:
    stage = "--stage" in sys.argv[1:]

    # 1 — index.py regenerates unconditionally.
    idx = run(INDEX_PY)
    echo(idx)
    if idx.returncode != 0:
        print(
            "\nrefresh.py: tools/docs/index.py could not regenerate — stopping before "
            "claims.py, which reads the corpus this would have rebuilt from.",
            file=sys.stderr,
        )
        return 1

    # 2 — claims.py --check, then the ratchet-only re-baseline. See the module
    # docstring: this order is load-bearing, not stylistic.
    claims = run(CLAIMS_PY, "--check")
    echo(claims)
    if claims.returncode != 0:
        print(
            "\nrefresh.py: tools/docs/claims.py --check found something NEW, SPREAD, or a "
            "pinned finding that stopped firing — claims-baseline.txt is left untouched. "
            "Fix the finding, or accept it deliberately with "
            "`uv run tools/docs/claims.py --write-baseline`.",
            file=sys.stderr,
        )
        return 1
    if "FELL" in claims.stdout:
        bless = run(CLAIMS_PY, "--write-baseline")
        echo(bless)
        if bless.returncode != 0:
            print(
                "\nrefresh.py: claims.py --write-baseline failed after a FELL — stopping.",
                file=sys.stderr,
            )
            return 1

    # 3 — rulings.py. Its exact half is the gate; the fuzzy half only reports.
    # The public tree carries neither the rulings nor this guard on them
    # (`ADR-010` §6.5), so its absence there is the tree's shape, not a fault.
    if RULINGS_PY.exists():
        rulings = run(RULINGS_PY)
        echo(rulings)
        if rulings.returncode != 0:
            return 1
    else:
        print("refresh.py: no tools/docs/rulings.py in this tree, so no rulings to guard.")

    # 4 — prose.py --check.
    prose = run(PROSE_PY, "--check")
    echo(prose)
    if prose.returncode != 0:
        return 1

    if stage:
        add = subprocess.run(
            ["git", "add", "--", *STAGED_PATHS],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if add.stdout:
            print(add.stdout, end="")
        if add.stderr:
            print(add.stderr, end="", file=sys.stderr)
        if add.returncode != 0:
            print("\nrefresh.py: `git add` of the staged paths failed.", file=sys.stderr)
            return 1

    print("\nrefresh.py: OK.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
