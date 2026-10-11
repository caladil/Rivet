#!/usr/bin/env python3
"""No operator-facing string may carry a run of spaces.

## What this catches, and why it is not a style preference

Rivet's house style wraps long messages with a `\\` line continuation, which
eats the newline **and** the next line's indentation:

    detail: "a long refusal that wraps \\
             across several lines",

When that backslash is lost - by a tool that rewrites the file, by a shell
heredoc, by anything that processes escapes on the way in - the newline goes
with it and the indentation is left behind as ordinary text. The literal becomes

    "a long refusal that wraps         across several lines"

**It compiles. Every test passes. Nothing catches it.** The only symptom is an
operator reading a sentence with a twelve-to-twenty-space hole in it, at the
worst moment they will have with this software.

A second, worse shape appears when the backslash survives as an escape:
`"...wraps \\n         across..."` renders a REAL newline and then the
indentation, which looks like deliberate formatting and is not.

## Why a repo-wide checker and not a unit test

`rivet-core/src/package/error.rs` has carried `no_refusal_carries_a_run_of_spaces`
since 2026-08-24. It works, and it covers **one enum in one crate** - so on
2026-09-02 sixty-five of these were found alive in twenty-nine files, none of
them reachable from that test's fixture. An invariant about every string in the
tree needs something that reads every string in the tree.

## The recorded cause was wrong, and that is worth stating

`error.rs` named `cargo fmt` as the cause: that it reflows a continued literal
onto one line. **It does not.** Tested directly on 2026-09-02 against rustfmt
1.9.0 with this repository's configuration - there is no `rustfmt.toml`, so
`format_strings` is false and string literals are left alone. The backslash
survived untouched. The real mechanism is anything that processes escapes while
writing the file, which in this project has meant agent tooling.

## Usage

    uv run tools/source/prose.py            # report
    uv run tools/source/prose.py --check    # exit 1 on anything not pinned

`tools/source/prose-baseline.txt` pins the places where a run of spaces is
DELIBERATE - column alignment, a test that searches its own source text, a doc
comment quoting the damage as an illustration. `--check` fails on anything new
**and on any pinned entry that no longer fires**, so repairing one edits the
baseline in the same change.

**A pin is `path | distinctive substring`, not `path:line`.** Line numbers were
tried first and were wrong within one commit: repairing a literal above a pinned
one turns a single line into three and every pin below it drifts, so the checker
fails over edits that have nothing to do with it. A substring survives that, and
a substring that stops matching is a pin worth re-reading anyway.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

BS = chr(92)
ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).resolve().parent / "prose-baseline.txt"

# A run of six or more spaces between two words, inside something that has a
# quote before it. Six rather than two: a doubled space is a typo, but six is
# the width of an indent and cannot be typed by accident.
#
# **`{` is in the trailing class, and leaving it out was a hole.** Added
# 2026-09-04 after a mangled line was read off a running board —
# `"... forgetting                  {relaunches} earlier relaunch(es)"` in
# `cmd/rivet-launcher` — that this file had been passing for days. The run was
# followed by a FORMAT PLACEHOLDER rather than by a letter, and `{` was not in
# the class, so the one construction that appears in almost every operator
# message in this repository was the one shape the guard could not see.
#
# That is this tool's own subject turned on itself: a check that reports
# `PASS` and cannot see the commonest case is worse than no check, because the
# `PASS` is what stops anybody looking. `AGENTS.md` §7 — an absence needs
# verifying as hard as a presence.
#
# **Capitals and digits are in both classes, and leaving them out was the same
# hole again.** Added 2026-09-13: `cmd/rivet`'s refusal for `rivet mcp` read
# `"It reads JSON-RPC                              from stdin"` and passed,
# because the run followed the `C` of an acronym. A lost continuation lands
# after whatever word the line happened to end on, and in an operator message
# that is as often `RPC`, `TLS` or a version number as a lowercase word.
RUN_IN_LITERAL = re.compile(r'"[^"]*[A-Za-z0-9,;)] {6,}[A-Za-z0-9{]')
RUN = re.compile(r"(?<=[A-Za-z0-9,;)]) {6,}(?=[A-Za-z0-9{])")
ESCAPED = re.compile(re.escape(BS + "n") + r" {5,}")


def tracked_rust_files() -> list[Path]:
    """Tracked `.rs` files only.

    `git ls-files` rather than a glob, so `target/`, scratch worktrees under
    `.claude/` and anything else untracked cannot contribute a finding that
    nobody can fix.
    """
    out = subprocess.run(
        ["git", "ls-files", "*.rs"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [ROOT / line for line in out.splitlines() if line]


def findings() -> list[tuple[str, int, str]]:
    found = []
    for path in tracked_rust_files():
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for number, line in enumerate(text.replace("\r\n", "\n").split("\n"), 1):
            if RUN_IN_LITERAL.search(line) or ESCAPED.search(line):
                where = path.relative_to(ROOT).as_posix()
                # The WHOLE line, not a truncation: a pin's substring may sit
                # past any cut-off, and matching against a shortened copy would
                # silently fail to find it.
                found.append((where, number, line.strip()))
    return found


def pinned() -> list[tuple[str, str]]:
    """`path | substring` pairs, in file order.

    Substring rather than line number: see the module docstring. A pin is
    `cmd/rivet-image/src/main.rs | # label`, which keeps matching when the
    lines above it move.
    """
    if not BASELINE.exists():
        return []
    out = []
    for line in BASELINE.read_text(encoding="utf-8").splitlines():
        # Only a WHOLE-line comment is a comment. A trailing `#` is not
        # stripped, because `# label` is itself one of the substrings pinned
        # here - a table header - and stripping it would leave the pin empty
        # and matching everything.
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        where, _, needle = line.partition("|")
        if needle.strip():
            out.append((where.strip(), needle.strip()))
    return out


def main() -> int:
    check = "--check" in sys.argv
    found = findings()
    allowed = pinned()

    def is_pinned(where: str, text: str) -> bool:
        return any(w == where and n in text for w, n in allowed)

    fresh = [f for f in found if not is_pinned(f[0], f[2])]
    stale = sorted(
        f"{w} | {n}"
        for w, n in allowed
        if not any(w == where and n in text for where, _, text in found)
    )

    for where, number, text in fresh:
        print(f"{where}:{number}: a run of spaces in a literal")
        print(f"    {text}")

    if stale:
        print("\npinned but no longer firing - remove these from the baseline:")
        for s in stale:
            print(f"  {s}")

    print(
        f"\n== prose - {len(fresh)} unpinned finding(s), "
        f"{len(allowed)} pinned, {len(stale)} stale pin(s)"
    )
    if not check:
        return 0
    if fresh or stale:
        print(
            "\nFAILED. A run of spaces reaches an operator as a hole in a sentence.\n"
            "Restore the continuation: end the line with a backslash and indent the\n"
            "next one. If the run is DELIBERATE - column alignment, a source-text\n"
            "search - pin it in tools/source/prose-baseline.txt with a reason."
        )
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
