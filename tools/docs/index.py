# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Build the section index for `docs/` — `ADR-029` §16.5.

*(This module cited ~~`PLAN.md` §101.5~~ four times until 2026-08-29. `PLAN.md`
§101 is a pointer stub with no children, so that number is a section of no document in
the corpus and **this index could not resolve its own justification.** The
content is `ADR-029` §16.5, and `PLAN.md`'s move table now says so. Found by
`tools/docs/claims.py`, which is this file's sibling.)*

**Why a SECTION index and not a search index.** `ADR-029` §16.5 measured the alternative
on the board: reading and scanning the whole 2.4 MB corpus off `erofs` costs
about 20 ms cold and ~2 ms warm, because the page cache already does the
pre-caching. A search index would save perhaps 10 ms against a process spawn and
a JSON serialisation that each cost more. It optimises the wrong resource.

**The scarce resource is the reader's context, not the board's CPU.** A grep
returning two hundred lines across thirty-seven files costs the asking agent
enormously more than it costs the Pi. So this indexes *where things are said*,
not *what words they contain* — and that works because this corpus has a
property most do not: it is rigorously structured, and **this project cites by
section and never by line.** Every commit message of 2026-08-29 says `ADR-029`
§9.3 or `ADR-012` §7.4.4. An index of headings lets a query answer in those terms
and let the caller fetch exactly the section named.

**`--check` is the drift guard.** An index is a second representation of content
that lives somewhere else, which is `ADR-011`'s problem — the same one
`ADR-029` §16.1
closed for the manual by making it a member of the signed payload. Here the
guard is cheaper: regenerate and compare. Committing a stale index would be
worse than having none, because it would confidently name the wrong lines.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path

# `RIVET_DOCS_ROOT` is an opt-in override for `tools/docs/section.py`'s tests,
# which point this tool at a fixture tree in the scratchpad rather than the
# real repository — so a test can regenerate a fixture's own index without
# ever writing into `docs/`. Unset, behaviour is exactly what it always was.
#
# **It announces itself when set**, for `claims.py`'s reason: an index silently
# generated from a fixture tree would pass `--check` while describing nothing in
# this repository, and the exit code could not say so.
_ROOT_OVERRIDE = os.environ.get("RIVET_DOCS_ROOT")
if _ROOT_OVERRIDE:
    print(
        f"index.py: RIVET_DOCS_ROOT is set — reading and writing {_ROOT_OVERRIDE}, "
        f"NOT this repository.",
        file=sys.stderr,
    )
ROOT = Path(_ROOT_OVERRIDE) if _ROOT_OVERRIDE else Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
INDEX_MD = DOCS / "INDEX.md"
INDEX_JSON = DOCS / "index.json"

# `## 9. Section` in the ADRs and SPECs; `## §94 — Title` in PLAN.md; and the
# `### 9.3.1` / `### §94.7` forms of both. The number is optional because some
# headings genuinely have none, and a heading without one is still a location.
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
NUMBER = re.compile(r"^§?\s*(\d+(?:\.\d+)*)\s*[.—:–-]?\s*(.*)$")

# Struck-through headings mean superseded, not deleted -- the house rule is that
# the record shows its own history. They stay in the index, flagged, because
# "where was that ruled before it was overturned" is a question worth answering.
STRUCK = re.compile(r"~~(.+?)~~")


def sections(path: Path) -> list[dict]:
    lines = path.read_text(encoding="utf-8").splitlines()
    found: list[dict] = []
    for i, line in enumerate(lines, start=1):
        m = HEADING.match(line)
        if not m:
            continue
        level, raw = len(m.group(1)), m.group(2)
        title = raw.strip()
        superseded = bool(STRUCK.search(title))
        number, rest = None, title
        nm = NUMBER.match(STRUCK.sub(r"\1", title).lstrip("#").strip())
        if nm:
            number, rest = nm.group(1), nm.group(2).strip()
        # A tombstone (`AGENTS.md` §2 rule 6): a numbered heading whose title,
        # after the number, starts `Retired ` — e.g. `### 5.4 Retired
        # 2026-09-07: superseded by X. See ADR-046 §3.` The section is not
        # gone; every citation into it still resolves, and this flag is how a
        # reader (and `claims.py`'s `retired-citations` check) tells a live
        # section from a marker left in its place.
        retired = bool(number) and rest.startswith("Retired ")
        found.append(
            {
                "level": level,
                "number": number,
                "title": rest or title,
                "line": i,
                "superseded": superseded,
                "retired": retired,
            }
        )
    # A section runs to the line before the next heading at the same level or
    # shallower; the last one runs to the end of the file. Ranges are what make
    # this fetchable rather than merely locatable.
    for n, sec in enumerate(found):
        end = len(lines)
        for later in found[n + 1 :]:
            if later["level"] <= sec["level"]:
                end = later["line"] - 1
                break
        sec["until"] = end
    return found


def commit() -> str | None:
    """The commit these documents were read at, or `None` outside a checkout.

    **Absence must mean absence.** An index that cannot say which commit it was
    built from says so with `null` rather than omitting the key or inventing a
    plausible value — `AGENTS.md` §5. A reader that cannot tell whether the
    manual matches the running binary needs to KNOW it cannot tell.
    """
    import subprocess

    try:
        out = subprocess.run(
            ["git", "-C", str(DOCS.parent), "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=10, check=False,
        )
        return out.stdout.strip() or None if out.returncode == 0 else None
    except (OSError, subprocess.SubprocessError):
        return None


def build() -> dict:
    """The index, and it is **self-describing on purpose**.

    `ADR-029` §16 ships this as a member of the signed payload and the
    catalogue's `system-manual` field points **at this file** rather than at the
    directory holding it. That is the whole discovery path for a cold agent: read
    the catalogue, fetch the one path it names, and everything else — the root,
    the commit, every document and every section — is in the answer.

    Pointing at the directory instead would require the reader to list it, notice
    a file called `index.json`, and *infer* that it is the map. **A convention
    that has to be guessed is not discoverable**, and guessing conventions is
    precisely what the cold agent this exists for cannot do.
    """
    docs = {}
    for path in sorted(DOCS.glob("*.md")):
        if path.name == "INDEX.md":
            continue
        docs[path.name] = sections(path)
    return {
        "schema": 1,
        "root": "docs",
        "commit": commit(),
        "files": len(docs),
        "sections": sum(len(v) for v in docs.values()),
        "cite": "Sections are addressed as `<file> §<number>`; `line`/`until` are "
                "1-based inclusive line numbers within `<file>`, so a section can be "
                "FETCHED and not merely located.",
        "documents": docs,
    }


def render(index: dict) -> str:
    docs = index["documents"]
    total = index["sections"]
    out = [
        "# `docs/` — section index",
        "",
        "* **Status:** Generated. Do not edit by hand.",
        "* **Generated by:** `uv run tools/docs/index.py`",
        "* **Guard:** `uv run tools/docs/index.py --check` fails if this is stale.",
        "",
        "---",
        "",
    ]
    out += [
        "## What this is, and what it is deliberately not",
        "",
        "**It indexes where things are SAID, not what words they contain.**",
        "`ADR-029` §16.5 measured the alternative on real hardware: scanning the",
        "whole corpus costs ~20 ms cold and ~2 ms warm, so a search index would",
        "save less time than the process spawn around it. What is scarce is the",
        "**reader's context** — a grep returning two hundred lines across",
        "thirty-seven files is expensive for the asker and free for the machine.",
        "",
        "This works because this project **cites by section and never by line**.",
        "Ask it where something is, then read that section.",
        "",
        f"**{total} sections across {len(docs)} files.** `~~struck~~` headings are",
        "superseded rulings, kept because the record shows its own history and",
        "*\"where was this ruled before it was overturned\"* is a real question.",
        "",
        "---",
        "",
    ]
    for name, secs in docs.items():
        if not secs:
            continue
        out.append(f"## `{name}`")
        out.append("")
        out.append("| Section | Title | Lines |")
        out.append("| :-- | :-- | --: |")
        for s in secs:
            if s["level"] == 1:
                continue
            num = s["number"] or ""
            mark = " ~~superseded~~" if s["superseded"] else ""
            mark += " **RETIRED**" if s.get("retired") else ""
            title = s["title"].replace("|", "\\|")
            out.append(f"| {num} | {title}{mark} | {s['line']}–{s['until']} |")
        out.append("")
    return "\n".join(out) + "\n"


def main() -> int:
    index = build()
    md = render(index)
    js = json.dumps(index, indent=2, ensure_ascii=False) + "\n"
    if "--check" in sys.argv:
        # **`commit` is compared loosely, and it has to be.** It records the
        # commit the index was BUILT at, and that value cannot be known until
        # after the commit that carries the index — so a strict comparison
        # failed on every commit that did not touch `docs/`, for ever, which is
        # not drift. Found 2026-08-29 by `tools/docs/claims.py`'s own baseline
        # needing this to pass. **A guard that cries wolf gets switched off**,
        # and a guard nobody runs catches nothing: `AGENTS.md` §10.5 records
        # twelve clippy errors shipping inside a change reported as "clippy
        # zero" for exactly this reason.
        #
        # What is still checked strictly is every section, line and range —
        # which is the drift this exists to catch. The commit is reported.
        stale = []
        if not INDEX_MD.exists() or INDEX_MD.read_text(encoding="utf-8") != md:
            stale.append(str(INDEX_MD))
        stored = None
        if INDEX_JSON.exists():
            try:
                stored = json.loads(INDEX_JSON.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                stored = None
        expected = json.loads(js)
        was = stored.get("commit") if stored else None
        if stored is None or {**stored, "commit": None} != {**expected, "commit": None}:
            stale.append(str(INDEX_JSON))
        if stale:
            print("STALE, regenerate with `uv run tools/docs/index.py`:", file=sys.stderr)
            for s in stale:
                print(f"  {s}", file=sys.stderr)
            return 1
        now = expected["commit"]
        drift = "" if was == now else f"; built at {(was or 'nothing')[:8]}, HEAD is {(now or 'unknown')[:8]}"
        print(f"index is current — {index['sections']} sections{drift}")
        return 0
    INDEX_MD.write_text(md, encoding="utf-8")

    # **The `commit` field alone is NOT a reason to rewrite this file**, and
    # skipping it is what stops a pre-commit hook churning it forever.
    #
    # `commit` records the tree the line ranges were generated from. It changes
    # on every commit by construction, so a hook that regenerates unconditionally
    # rewrites `index.json` in *every* commit — which is the exact cost the hook
    # was adopted to remove (`.rulings/` R-153, Q5: it appeared in 287 of 1,289
    # commits and the hook would have taken that to all of them).
    #
    # `--check` has always ignored this field when deciding staleness, because a
    # file whose only difference is the commit hash describes the same headings
    # at the same lines and is equally valid. The write path now agrees with it.
    # The field therefore comes to mean **the last commit at which the index's
    # CONTENT changed**, which is the more useful of the two readings.
    stored = None
    if INDEX_JSON.exists():
        try:
            stored = json.loads(INDEX_JSON.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            stored = None
    fresh = json.loads(js)
    unchanged = stored is not None and {**stored, "commit": None} == {**fresh, "commit": None}

    if unchanged:
        print(f"{INDEX_MD.name} written; {INDEX_JSON.name} unchanged — "
              f"{index['sections']} sections across {index['files']} files")
        return 0

    INDEX_JSON.write_text(js, encoding="utf-8")
    print(f"wrote {INDEX_MD.name} and {INDEX_JSON.name} — "
          f"{index['sections']} sections across {index['files']} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
