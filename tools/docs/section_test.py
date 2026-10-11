# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Tests for `tools/docs/section.py`, run with `uv run tools/docs/section_test.py`.

**Every fixture lives under the platform's temp directory, never under `docs/`.**
The one read of a real document is `build_template()` copying
`docs/ADR-021-Time.md` byte-for-byte as the seed content — reading it is not
retiring it. Every `retire` call in this file passes `--root` at a fixture
copy, and `section.py`'s own `--root` handling is what keeps `index.py` and
`claims.py` off the real `docs/` tree too (`RIVET_DOCS_ROOT`, set only for the
two subprocess calls `section.py` itself makes).

`docs/ADR-021-Time.md` §4 is the fixture's live target throughout: it has two
`### 4.x` children and one `#### 4.2.1` grandchild — three sub-headings, so
retiring §4 is the "three sub-headings, four tombstones" case the acceptance
test asks for, using the section as it is actually numbered rather than a
hand-trimmed stand-in.
"""

from __future__ import annotations

import argparse
import hashlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
SECTION_PY = HERE / "section.py"

sys.path.insert(0, str(HERE))
import section  # noqa: E402
from index import sections  # noqa: E402

SCRATCH = Path(tempfile.gettempdir()) / "rivet-section-test"
TEMPLATE = SCRATCH / "section-fixture-template"
WORK = SCRATCH / "section-test-work"

CITER_RS = (
    "// This module implements the boot-time step described in\n"
    "// `ADR-021` §4, and specifically the immediate step at boot\n"
    "// described in `ADR-021` §4.1.\n"
    "fn placeholder() {}\n"
)
PLAN_MD = "# PLAN\n\n## 1. Something\n\nBody text about the current plan.\n"


def build_template() -> None:
    """(Re)build the fixture template from the real `ADR-021-Time.md`.

    This is the ONLY read of a real document in this file, and it is a read —
    `shutil.copy2` — never a write back into `docs/`.
    """
    if TEMPLATE.exists():
        shutil.rmtree(TEMPLATE)
    (TEMPLATE / "docs").mkdir(parents=True)
    (TEMPLATE / "src").mkdir(parents=True)
    shutil.copy2(REPO_ROOT / "docs" / "ADR-021-Time.md", TEMPLATE / "docs" / "ADR-021-Time.md")
    (TEMPLATE / "docs" / "PLAN.md").write_text(PLAN_MD, encoding="utf-8", newline="\n")
    (TEMPLATE / "src" / "citer.rs").write_text(CITER_RS, encoding="utf-8", newline="\n")


def fresh_root(name: str) -> Path:
    dest = WORK / name
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(TEMPLATE, dest)
    return dest


def run_cli(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SECTION_PY), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Failure(AssertionError):
    pass


def expect(cond: bool, msg: str) -> None:
    if not cond:
        raise Failure(msg)


# --------------------------------------------------------------------------
# Test 1 — three sub-headings produce four tombstones, all four still resolve
# --------------------------------------------------------------------------


def test_four_tombstones_all_resolve() -> None:
    root = fresh_root("t1")
    doc = root / "docs" / "ADR-021-Time.md"

    result = run_cli(
        "retire", "ADR-021 §4",
        "--reason", "superseded by the proxy gate",
        "--to", "ADR-046 §3",
        "--root", str(root),
        "--date", "2026-09-07",
    )
    expect(result.returncode == 0, f"expected exit 0, got {result.returncode}: {result.stderr}")

    secs = sections(doc)
    by_number = {s["number"]: s for s in secs if s["number"] in ("4", "4.1", "4.2", "4.2.1")}
    expect(len(by_number) == 4, f"expected 4 tombstoned headings, found {sorted(by_number)}")
    for number, s in by_number.items():
        expect(s["retired"], f"§{number} was not marked retired")
        expect(
            s["title"].startswith("Retired 2026-09-07: superseded by the proxy gate."),
            f"§{number}'s tombstone reads wrong: {s['title']!r}",
        )

    # "still resolve" — the real claims.py, pointed at the fixture, must not
    # report any of the four as a dangling citation target.
    counted = section.citation_count(root)
    expect(counted >= 0, "citation_count refused to run")
    findings_at_citer = [
        w for w, _ in section.retired_citation_lines(root) if w.startswith("src/citer.rs")
    ]
    expect(len(findings_at_citer) == 2, f"expected both citer.rs lines flagged retired, got {findings_at_citer}")


# --------------------------------------------------------------------------
# Test 2 — the citing .rs line appears in the listed output
# --------------------------------------------------------------------------


def test_citing_line_is_listed() -> None:
    root = fresh_root("t2")
    result = run_cli(
        "retire", "ADR-021 §4",
        "--reason", "superseded by the proxy gate",
        "--root", str(root),
        "--date", "2026-09-07",
    )
    expect(result.returncode == 0, f"expected exit 0: {result.stderr}")
    expect(
        "src/citer.rs:2" in result.stdout and "`ADR-021` §4" in result.stdout,
        f"citer.rs's §4 citation not in listed output:\n{result.stdout}",
    )
    expect(
        "src/citer.rs:3" in result.stdout and "§4.1" in result.stdout,
        f"citer.rs's §4.1 citation not in listed output:\n{result.stdout}",
    )


# --------------------------------------------------------------------------
# Test 3 — a CRLF file round-trips as CRLF, with no mixed endings
# --------------------------------------------------------------------------


def test_crlf_round_trip() -> None:
    root = fresh_root("t3")
    doc = root / "docs" / "ADR-021-Time.md"
    # Force CRLF regardless of how this checkout's own line endings came in,
    # so the test does not depend on `core.autocrlf` doing it by accident.
    normalised = doc.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
    doc.write_bytes(normalised)

    result = run_cli(
        "retire", "ADR-021 §4",
        "--reason", "superseded by the proxy gate",
        "--root", str(root),
        "--date", "2026-09-07",
    )
    expect(result.returncode == 0, f"expected exit 0: {result.stderr}")

    raw = doc.read_bytes()
    expect(b"\r\n" in raw, "result lost its CRLF endings entirely")
    mixed = any(raw[i : i + 1] == b"\n" and raw[i - 1 : i] != b"\r" for i in range(1, len(raw)))
    expect(not mixed, "result has a bare LF not preceded by CR — mixed endings")
    stray_cr = any(
        raw[i : i + 1] == b"\r" and raw[i + 1 : i + 2] != b"\n" for i in range(len(raw) - 1)
    )
    expect(not stray_cr, "result has a lone CR not followed by LF — mixed endings")


# --------------------------------------------------------------------------
# Test 4 — PLAN.md is refused, unchanged
# --------------------------------------------------------------------------


def test_plan_md_refused() -> None:
    root = fresh_root("t4")
    plan = root / "docs" / "PLAN.md"
    before = sha256(plan)

    result = run_cli(
        # Built by concatenation, not as one literal string: writing the PLAN
        # filename immediately next to a section mark in this file's own
        # source text would itself be the citation `plan-citations` (rule 5)
        # exists to catch.
        "retire", "PLAN.md" + " §1",
        "--reason", "test",
        "--root", str(root),
        "--date", "2026-09-07",
    )
    expect(result.returncode == 1, f"expected exit 1, got {result.returncode}")
    expect("refused" in result.stderr, f"no refusal message: {result.stderr}")
    expect(sha256(plan) == before, "PLAN.md was modified despite the refusal")


# --------------------------------------------------------------------------
# Test 5 — a planted post-write failure restores every touched file
# --------------------------------------------------------------------------


def test_planted_failure_restores_everything() -> None:
    root = fresh_root("t5")
    doc = root / "docs" / "ADR-021-Time.md"
    index_md = root / "docs" / "INDEX.md"
    index_json = root / "docs" / "index.json"

    before_doc = doc.read_bytes()
    expect(not index_md.exists(), "fixture already has an INDEX.md — test assumption broken")
    expect(not index_json.exists(), "fixture already has an index.json — test assumption broken")

    real_before = section.citation_count(root)
    calls = {"n": 0}

    def rising(_root: Path) -> int:
        calls["n"] += 1
        return real_before if calls["n"] == 1 else real_before + 1

    original = section.citation_count
    section.citation_count = rising  # type: ignore[assignment]
    try:
        ns = argparse.Namespace(
            address="ADR-021 §4",
            reason="superseded by the proxy gate",
            to=None,
            dry_run=False,
            root=str(root),
            date="2026-09-07",
            backup_dir=str(WORK / "backup"),
        )
        rc = section.cmd_retire(ns)
    finally:
        section.citation_count = original  # type: ignore[assignment]

    expect(rc == 1, f"expected exit 1 on a planted rise, got {rc}")
    expect(calls["n"] == 2, f"expected citation_count called twice (before, after), got {calls['n']}")
    expect(doc.read_bytes() == before_doc, "ADR-021-Time.md was not restored byte-for-byte")
    expect(not index_md.exists(), "docs/INDEX.md should have been rolled back to nonexistent")
    expect(not index_json.exists(), "docs/index.json should have been rolled back to nonexistent")


# --------------------------------------------------------------------------
# Test 6 — --dry-run writes nothing
# --------------------------------------------------------------------------


def test_dry_run_writes_nothing() -> None:
    root = fresh_root("t6")
    doc = root / "docs" / "ADR-021-Time.md"
    before = sha256(doc)

    result = run_cli(
        "retire", "ADR-021 §4",
        "--reason", "superseded by the proxy gate",
        "--root", str(root),
        "--date", "2026-09-07",
        "--dry-run",
    )
    expect(result.returncode == 0, f"expected exit 0: {result.stderr}")
    expect("---" in result.stdout, "expected a unified diff on stdout")
    expect(sha256(doc) == before, "dry-run modified the fixture")
    expect(not (root / "docs" / "INDEX.md").exists(), "dry-run created docs/INDEX.md")
    expect(not (root / "docs" / "index.json").exists(), "dry-run created docs/index.json")


# Test 7 — an unnumbered heading is tombstoned without the word `None`
#
# The regression from this tool's first run against a real document. Every
# fixture above numbers every heading, which is why six passing tests did not
# catch a line reading `#### None Retired …`. This asserts the renderer
# directly rather than through a fixture, because the defect is in one
# interpolation and a fixture would test the parser as well as the bug.
def test_an_unnumbered_heading_gets_no_number() -> None:
    numbered = section.tombstone_line(4, "5.4", "superseded by the proxy gate", "2026-09-07", "ADR-046 §3")
    expect(
        numbered == "#### 5.4 Retired 2026-09-07: superseded by the proxy gate. See `ADR-046` §3.",
        f"a numbered tombstone reads wrong: {numbered!r}",
    )

    bare = section.tombstone_line(4, None, "superseded by the proxy gate", "2026-09-07", None)
    expect("None" not in bare, f"the literal word None reached the line: {bare!r}")
    expect(
        bare == "#### Retired 2026-09-07: superseded by the proxy gate.",
        f"an unnumbered tombstone reads wrong: {bare!r}",
    )

    # An empty string is the same case arriving by a different route.
    empty = section.tombstone_line(2, "", "moved", "2026-09-07", None)
    expect(empty == "## Retired 2026-09-07: moved.", f"an empty number reads wrong: {empty!r}")


TESTS = [
    test_four_tombstones_all_resolve,
    test_citing_line_is_listed,
    test_crlf_round_trip,
    test_plan_md_refused,
    test_planted_failure_restores_everything,
    test_dry_run_writes_nothing,
    test_an_unnumbered_heading_gets_no_number,
]


def main() -> int:
    build_template()
    WORK.mkdir(parents=True, exist_ok=True)
    failures = 0
    for test in TESTS:
        name = test.__name__
        try:
            test()
        except Failure as exc:
            print(f"FAIL {name}: {exc}")
            failures += 1
        except Exception as exc:  # pragma: no cover — an unexpected error is still a failure
            print(f"ERROR {name}: {exc!r}")
            failures += 1
        else:
            print(f"PASS {name}")
    print(f"\n{len(TESTS) - failures}/{len(TESTS)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
