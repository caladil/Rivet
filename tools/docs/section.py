# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write a tombstone over a retired section — `AGENTS.md` §2 rule 6.

Ruled 2026-09-07 (`.rulings/` R-153, Phase 3). The rule that section is
building for: a heading in an ADR or SPEC is never renumbered, and a section
whose content is retired is replaced by a one-line tombstone at the same
level and number, so every citation into it still resolves.

**Only `retire` is built.** The originally-specified four-verb tool
(`get`/`put`/`retire`/`move`) was deferred by the maintainer; `move` in
particular is the half judged capable of corrupting the corpus quietly by
rewriting citations, and citation-rewriting is deliberately out of scope here
— this tool never touches a citing site, only the section it retires.

    uv run tools/docs/section.py retire "ADR-021 §9.1" --reason "superseded by X"
    uv run tools/docs/section.py retire "ADR-021 §9.1" --reason "..." --to "ADR-046 §3"
    uv run tools/docs/section.py retire "ADR-021 §9.1" --reason "..." --dry-run

## What it does, in order

1. Resolves the address to a file and a section, using `index.py`'s own
   `sections()` parser — this file does not reimplement heading parsing.
2. Refuses a target that this project's headings-are-immortal rule does not
   apply to: `PLAN*.md` (a journal, cut and renumbered on purpose), `STATE.md`
   (rewritten wholesale on every suspend), anything under `.rulings/` (the
   maintainer's own words), `docs/INDEX.md` (generated), and anything under
   `vendor/`, `target/` or `.claude/` (not this project's own design record).
3. Replaces the named section AND every heading in its subtree with tombstone
   lines, one per heading, in original order, at the same level and number,
   with no body text between them.
4. Backs up every file the write will touch, byte-for-byte, before writing
   anything.
5. Writes the file back in its OWN line-ending convention — CRLF stays CRLF,
   LF stays LF — never mixed.
6. Regenerates `docs/INDEX.md` / `docs/index.json` and lists every explicit
   citation of a number just retired, without rewriting any of them.
7. Re-runs `claims.py`'s `citations` check and compares the count before and
   after. **If it rose, every touched file is restored byte-for-byte from the
   backup and the tool exits 1.** Retiring a section correctly cannot break a
   citation; a rise means the tombstones are wrong.

## `--root` and how the safety net reaches a fixture tree

`index.py` and `claims.py` are single-repository tools — their own `ROOT` is
`Path(__file__).resolve().parents[2]`, not a parameter. Both now read an
opt-in `RIVET_DOCS_ROOT` environment override (added alongside this file, and
otherwise inert) so this tool's tests can point the REAL `index.py` and the
REAL `claims.py` at a fixture tree outside the repository — the actual regenerator
and the actual citation checker, not a reimplementation of either — while
never touching `docs/`. `--root` sets that environment variable for the two
subprocess calls this tool makes; omit it and both point at this repository,
exactly as they always have.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import date, datetime, timezone
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
INDEX_PY = HERE / "index.py"
CLAIMS_PY = HERE / "claims.py"

sys.path.insert(0, str(HERE))
from index import sections  # noqa: E402  (import after sys.path fix-up, by design)

# Where every backup goes unless `--backup-dir` names another: a UTC-timestamped
# folder outside the repository being edited. A path is an argument, never a
# constant (`AGENTS.md` §8.3), so the default is the platform's own temp directory.
DEFAULT_BACKUP_ROOT = Path(tempfile.gettempdir()) / "rivet-section-backup"

TOMBSTONE_HEAD = re.compile(r"^~*\**\s*§?\s*(\d+[A-Z]?(?:\.\d+[A-Z]?)*)\s*$")


# --------------------------------------------------------------------------
# Address resolution
# --------------------------------------------------------------------------


@dataclass
class Address:
    doc_ref: str
    number: str


def parse_address(raw: str) -> Address:
    m = re.match(r"^(?P<doc>.+?)\s*§\s*(?P<num>\d+[A-Z]?(?:\.\d+[A-Z]?)*)\s*$", raw)
    if not m:
        raise ValueError(
            f"{raw!r} is not `<document> §<number>` — e.g. `ADR-021 §9.1`"
        )
    return Address(doc_ref=m.group("doc").strip().strip("`"), number=m.group("num"))


def resolve_document(root: Path, doc_ref: str) -> Path | None:
    """The file an address's document half names, or `None` if it does not.

    Mirrors `claims.py`'s `doc_key()` convention in reverse: `ADR-021` finds
    `docs/ADR-021-*.md`; anything ending `.md` is looked up directly, under
    `root` and under `root/docs`; anything containing a path separator is
    taken as a path relative to `root`.
    """
    if "/" in doc_ref or "\\" in doc_ref:
        candidate = root / doc_ref
        return candidate.resolve() if candidate.is_file() else None
    if doc_ref.lower().endswith(".md"):
        for base in (root, root / "docs"):
            candidate = base / doc_ref
            if candidate.is_file():
                return candidate.resolve()
        return None
    docs_dir = root / "docs"
    if not docs_dir.is_dir():
        return None
    matches = sorted(docs_dir.glob(f"{doc_ref}-*.md"))
    if len(matches) == 1:
        return matches[0].resolve()
    return None  # zero or ambiguous — the caller reports it as unresolvable


# --------------------------------------------------------------------------
# The refusal list — `AGENTS.md` §2 rule 6 applies to ADRs and SPECs, not to
# a journal, a snapshot, the maintainer's own words, or foreign/generated trees.
# --------------------------------------------------------------------------


def refuse_reason(root: Path, path: Path) -> str | None:
    try:
        rel = path.relative_to(root).as_posix()
    except ValueError:
        rel = path.as_posix()
    name = path.name
    parts = Path(rel).parts
    if name == "STATE.md":
        return "STATE.md is rewritten wholesale on every suspend, never hand-edited section by section"
    if re.match(r"^PLAN(-Archive-\d{2}[A-Za-z0-9.-]*)?\.md$", name):
        return "PLAN*.md is a journal — cut and renumbered on purpose; headings there are not immortal the way an ADR/SPEC's are"
    if ".rulings" in parts:
        return ".rulings/ holds the maintainer's own words verbatim and is never edited by a tool"
    if rel == "docs/INDEX.md":
        return "docs/INDEX.md is generated by index.py and is never hand-edited"
    if "vendor" in parts:
        return "vendor/ is foreign source, not this project's own design record"
    if "target" in parts:
        return "target/ is build output, not a document"
    if ".claude" in parts:
        return ".claude/ is tooling configuration, not part of the design record"
    return None


# --------------------------------------------------------------------------
# Line endings — detect, work in LF, write back in the file's own convention
# --------------------------------------------------------------------------


def detect_eol(raw: bytes) -> str:
    return "\r\n" if b"\r\n" in raw else "\n"


def has_mixed_eol(text: str, eol: str) -> bool:
    """True if `text` contains a line ending other than `eol`."""
    if eol == "\r\n":
        return re.search(r"(?<!\r)\n", text) is not None
    return "\r" in text


# --------------------------------------------------------------------------
# Tombstones
# --------------------------------------------------------------------------


def format_to(to: str) -> str:
    """`ADR-046 §3` -> `` `ADR-046` §3 ``, matching the house citation style.

    Left alone if it already carries a backtick, or names nothing this
    pattern recognises as a document — the caller's wording is used verbatim
    rather than guessed at.
    """
    to = to.strip()
    if "`" in to:
        return to
    m = re.match(r"^(ADR-\d{3}|SPEC-\d{2}|[A-Za-z0-9_.-]+\.md)\b", to)
    if not m:
        return to
    return f"`{m.group(1)}`{to[m.end():]}"


def tombstone_line(
    level: int, number: str | None, reason: str, iso_date: str, to: str | None
) -> str:
    """One tombstone, at the retired heading's own level and number.

    # An unnumbered heading has no number, and `None` is not a number

    `index.py` parses a heading's number and yields `None` for one that carries
    none — a `#### Why this is not a crate` under a numbered parent, say. This
    interpolated that straight into the line and wrote the literal word
    **`None`** where the number belongs:

        #### None Retired 2026-09-07: superseded. See `ADR-036` §2.2.

    Found on this tool's first run against a real document (`ADR-013`,
    2026-09-07); every fixture until then happened to number every heading.
    The safety net did its job — citations were level before and after, because
    a citation into an unnumbered heading is not a thing anyone can write — so
    nothing was broken, and the output was still wrong in a way a reader would
    have had to repair by hand.

    An unnumbered heading is tombstoned **without** a number rather than with a
    placeholder. `AGENTS.md` §2 rule 6 requires the tombstone to hold the number
    a citation might resolve to; where there is no number, there is nothing to
    hold, and inventing one would be worse than omitting it.
    """
    clause = f" See {format_to(to)}." if to else ""
    reason = reason.strip().rstrip(".")
    label = f"{number} " if number else ""
    return f"{'#' * level} {label}Retired {iso_date}: {reason}.{clause}"


def subtree_of(secs: list[dict], target: dict) -> list[dict]:
    idx = secs.index(target)
    out = [target]
    for later in secs[idx + 1 :]:
        if later["level"] <= target["level"]:
            break
        out.append(later)
    return out


# --------------------------------------------------------------------------
# The two subprocess calls that ARE the safety net — the real `index.py` and
# the real `claims.py`, redirected at `root` via `RIVET_DOCS_ROOT`.
# --------------------------------------------------------------------------


def _run(script: Path, args: list[str], root: Path) -> subprocess.CompletedProcess[str]:
    env = dict(os.environ, RIVET_DOCS_ROOT=str(root), PYTHONIOENCODING="utf-8")
    return subprocess.run(
        [sys.executable, str(script), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=env,
    )


def regenerate_index(root: Path) -> None:
    result = _run(INDEX_PY, [], root)
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise RuntimeError("tools/docs/index.py could not regenerate against the new content")


def citation_count(root: Path) -> int:
    result = _run(CLAIMS_PY, ["--only", "citations", "--json"], root)
    data = json.loads(result.stdout)
    return sum(1 for f in data["findings"] if f["check"] == "citations")


def retired_citation_lines(root: Path) -> list[tuple[str, str]]:
    result = _run(CLAIMS_PY, ["--only", "retired-citations", "--json"], root)
    data = json.loads(result.stdout)
    return [
        (f["where"], f["evidence"]) for f in data["findings"] if f["check"] == "retired-citations"
    ]


# --------------------------------------------------------------------------
# Backup and restore
# --------------------------------------------------------------------------


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")


@dataclass
class Touched:
    path: Path
    existed: bool
    backup: Path | None


def backup_files(paths: list[Path], root: Path, backup_dir: Path) -> list[Touched]:
    touched: list[Touched] = []
    for p in paths:
        existed = p.exists()
        backup: Path | None = None
        if existed:
            rel = p.relative_to(root)
            backup = backup_dir / rel
            backup.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(p, backup)
        touched.append(Touched(path=p, existed=existed, backup=backup))
    return touched


def restore(touched: list[Touched]) -> None:
    for t in touched:
        if t.existed:
            assert t.backup is not None
            t.path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(t.backup, t.path)
        elif t.path.exists():
            t.path.unlink()


# --------------------------------------------------------------------------
# `retire`
# --------------------------------------------------------------------------


def cmd_retire(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve() if args.root else REPO_ROOT
    iso_date = args.date or date.today().isoformat()

    try:
        address = parse_address(args.address)
    except ValueError as exc:
        print(f"bad address: {exc}", file=sys.stderr)
        return 2

    path = resolve_document(root, address.doc_ref)
    if path is None:
        print(
            f"cannot resolve document {address.doc_ref!r} under {root} — no such file, "
            f"or {address.doc_ref!r} matches zero or more than one `docs/{address.doc_ref}-*.md`",
            file=sys.stderr,
        )
        return 2

    refusal = refuse_reason(root, path)
    if refusal is not None:
        rel = path.relative_to(root).as_posix()
        print(f"refused: {rel} — {refusal}", file=sys.stderr)
        return 1

    raw = path.read_bytes()
    eol = detect_eol(raw)
    text = raw.decode("utf-8")
    trailing_newline = text.endswith(("\n", "\r\n"))
    lines = text.splitlines()

    secs = sections(path)
    target = next((s for s in secs if s["number"] == address.number), None)
    if target is None:
        print(f"no §{address.number} in {path.relative_to(root).as_posix()}", file=sys.stderr)
        return 2

    subtree = subtree_of(secs, target)
    tombstones = [
        tombstone_line(s["level"], s["number"], args.reason, iso_date, args.to) for s in subtree
    ]
    start, end = target["line"], target["until"]
    new_lines = lines[: start - 1] + tombstones + lines[end:]
    new_text = eol.join(new_lines) + (eol if trailing_newline else "")

    if has_mixed_eol(new_text, eol):
        print(
            f"post-write check failed: {path.relative_to(root).as_posix()} would come out with "
            f"mixed line endings — refusing to write",
            file=sys.stderr,
        )
        return 1

    if args.dry_run:
        diff = difflib.unified_diff(
            [l + "\n" for l in lines],
            [l + "\n" for l in new_lines],
            fromfile=str(path),
            tofile=f"{path} (retired)",
        )
        sys.stdout.writelines(diff)
        return 0

    index_md = root / "docs" / "INDEX.md"
    index_json = root / "docs" / "index.json"

    before = citation_count(root)

    backup_root = Path(args.backup_dir) if args.backup_dir else DEFAULT_BACKUP_ROOT
    backup_dir = backup_root / utc_timestamp()
    touched = backup_files([path, index_md, index_json], root, backup_dir)

    path.write_bytes(new_text.encode("utf-8"))

    try:
        regenerate_index(root)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        restore(touched)
        return 1

    for where, evidence in retired_citation_lines(root):
        print(f"{where}: {evidence}")

    after = citation_count(root)
    print(f"citations before: {before}, after: {after}")

    if after > before:
        restore(touched)
        print(
            f"citation count ROSE ({before} -> {after}) — every touched file restored "
            f"byte-for-byte from {backup_dir}",
            file=sys.stderr,
        )
        return 1

    return 0


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="section.py",
        description="Write tombstones over retired ADR/SPEC sections (AGENTS.md §2 rule 6).",
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    retire = sub.add_parser("retire", help="Tombstone a section and its subtree.")
    retire.add_argument("address", help='e.g. "ADR-021 §9.1"')
    retire.add_argument("--reason", required=True)
    retire.add_argument("--to", default=None, help='e.g. "ADR-046 §3"')
    retire.add_argument("--dry-run", action="store_true")
    retire.add_argument("--root", default=None, help="fixture root, for tests")
    retire.add_argument("--date", default=None, help="override today's date (YYYY-MM-DD)")
    retire.add_argument(
        "--backup-dir",
        default=None,
        help=f"where the pre-edit backup goes (default: {DEFAULT_BACKUP_ROOT})",
    )
    retire.set_defaults(func=cmd_retire)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
