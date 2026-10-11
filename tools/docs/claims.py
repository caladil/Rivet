# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check the claims this record makes about itself and about the tree.

**Why this exists.** On 2026-08-29 a cold agent was given this repository and a
board and told to deploy an application. It read `ADR-015` §3.2 — *"gets
`/state/app/<name>`, mode 700"* — wrote code against it, and the board answered
`EACCES`. The correct answer was four hundred lines further down the same file,
in §5.1.1: the writable directory is `/state/app/<name>/data`. Its own words:
*"not a gap I noticed, but a confident sentence I followed."*

**A single wrong sentence is a bug. A record with no way to notice when a claim
stops being true is a defect in the system that produces the record.**
`AGENTS.md` §2's house rule is that a superseded claim is struck in place citing
what replaced it, and until this tool existed nothing checked that anyone had.

This is `tools/docs/index.py`'s sibling and follows its shape exactly: derive the
answer from the tree, compare it to what is written down, and fail loudly.
`index.py` guards the index against the documents; this guards the documents
against each other and against the source.

**What it deliberately does NOT do, stated because a checker that hides its
blind spots is worse than none** (`AGENTS.md` §7 — a claim of absence needs
re-checking as hard as a claim of presence):

* It cannot read prose for meaning. Two sections that contradict each other in
  words rather than in a number or a path are invisible to it.
* It cannot tell a claim that was true and rotted from one that was always
  wrong. Both look the same from here.
* `--check` proves the checkable claims are consistent. It never proves the
  record is *correct* — `AGENTS.md` §6: until it runs, it isn't working.

Every derivation refuses by name rather than returning a plausible value
(`AGENTS.md` §3, §5). A derivation that stops working is reported as a finding,
so this cannot silently pass by having quietly stopped deriving anything.

    uv run tools/docs/claims.py                   # report everything, exit 0
    uv run tools/docs/claims.py --check           # exit 1 on anything NEW
    uv run tools/docs/claims.py --write-baseline  # pin what is tabled
    uv run tools/docs/claims.py --only citations  # one check, not a gate
    uv run tools/docs/claims.py --only ghosts     # prose that outlived its subject

**`--check` is a gate and not a wall.** Findings stay open because their
*repair* is a maintainer's ruling — they are tabled in `PLAN.md` §102.4 (in
`PLAN-Archive-08` since 2026-09-02; the pointer table is what keeps that
citation resolving) and pinned in `claims-baseline.txt`. **The number is
deliberately not written here** — it was "thirty-two" while the baseline held
forty-five, which is this tool's own `ADR-011` finding pointed at its docstring. `--check` fails on a finding that is new,
on one that spread, **and on a pinned finding that no longer fires** — because a
baseline nobody maintains is a second representation of the record's state,
which is `ADR-011`'s problem one level up.

**Four of the ten checks are RATCHETS.** `plan-citations`, `rot-markers`,
`citations` and `deep-citations` are baselined at a backlog nobody is going to
clear: they fail when a number goes **up** and print good news when it goes
down. `ghosts`, `narrative` and `retired-citations` are **warnings** and gate
nothing at all. See `RATCHET_ONLY` and `WARN_ONLY` at the foot of this file for
why each is where it is — including the ones that were a judgement call and
not a ruling.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

# The corpus is UTF-8 and full of `§` and em dashes. A Windows console defaults
# to cp1252 and would mangle every finding this prints — which is `AGENTS.md`
# §10.5's complaint in another costume: a check whose output cannot be read.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

# `RIVET_DOCS_ROOT` is an opt-in override for `tools/docs/section.py`'s tests
# — see index.py's identical override for why. Unset, behaviour is unchanged.
#
# **It ANNOUNCES ITSELF when it is set, and that is not decoration.** This tool
# exists because a confident sentence about the wrong thing is worse than no
# sentence, and an environment variable that silently redirected every check at
# a different tree would be exactly that failure wearing this tool's own face:
# `--check` would exit 0 having examined a fixture in the scratchpad, and the
# exit code alone could not tell anyone. A checker that cannot be quietly
# pointed elsewhere is worth two lines.
_ROOT_OVERRIDE = os.environ.get("RIVET_DOCS_ROOT")
if _ROOT_OVERRIDE:
    print(
        f"claims.py: RIVET_DOCS_ROOT is set — checking {_ROOT_OVERRIDE}, "
        f"NOT this repository. Every number below is about that tree.",
        file=sys.stderr,
    )
ROOT = Path(_ROOT_OVERRIDE) if _ROOT_OVERRIDE else Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"

# Documents that can be the target of a `§` citation. `docs/` plus the two the
# corpus cites by name from outside it.
EXTRA_DOCS = ("AGENTS.md", "README.md")

# Source trees searched for path literals and for citations written in code.
# `target/` is build output and `.git/` is not text.
#
# **`vendor/` is skipped because its `§` citations are somebody else's.** Since
# 2026-09-04 it holds `ixr-smb-server`, adopted under `.rulings/` R-134, whose
# comments cite **MS-SMB2** — `§2.2.1.2`, `§3.1.4.1`, `§3.3.5.2.4` and sixty
# more. Those are section numbers in Microsoft's protocol specification and this
# checker resolves citations against Rivet's `docs/` corpus, so every one of
# them reads as a citation of a document that does not exist. Sixty-four
# findings arrived the moment the crate was tracked, none of them about this
# project.
#
# The rule this follows is the one already applied to `cmd/cargo-rivet`'s test
# fixtures: a tree whose contents are INPUT rather than this project's own prose
# is not this checker's subject. What is NOT skipped is `vendor/*/VENDORING.md`
# — that file is Rivet's own writing about the crate, cites Rivet documents, and
# is checked like any other.
SKIP_DIRS = {".git", "target", ".claude", "node_modules"}
SKIP_PREFIXES = ("vendor/",)
SOURCE_SUFFIXES = {".rs", ".toml", ".sh", ".py", ".nu", ".c", ".h"}


# --------------------------------------------------------------------------
# Section numbers, as this project writes them
# --------------------------------------------------------------------------

# A heading's number: `## 9. Title`, `### 9.3.1 Title`, `## §94 — Title`,
# `## 8A. Break-glass`, `#### (g) ...`. The alpha suffix is real and is used
# where a section was inserted after its neighbours were numbered — `ADR-035`
# has `4A`, `8A`, `9A` and `9B`.
HEADING = re.compile(r"^(#{1,6})\s+(.*)$")
# The alpha suffix is UPPERCASE on purpose: `4A`, `8A`, `9A`, `9B` are real
# section numbers, and a lowercase tail is English — `§23s` is a plural, not a
# section, and reading it as one was this checker's first false positive.
HEAD_NUM = re.compile(r"^~*\**\s*§?\s*(\d+[A-Z]?(?:\.\d+[A-Z]?)*)")

# A citation in running text. The number stops at the first character that
# cannot be part of one, so `§9.3's`, `§9.2 step 3` and `§8.10.13,` all yield
# just the number.
CITE = re.compile(r"§\s*(\d+[A-Z]?(?:\.\d+[A-Z]?)*)")

# `RFC 4253 §6.6` and `RFC 8032 §7.1` are citations of somebody else's document
# and this corpus is not their index. Anything whose `§` is introduced by an
# external standard is skipped rather than resolved against Rivet's own
# sections, which is where the first draft of this checker produced its
# loudest nonsense.
EXTERNAL = re.compile(
    r"(?:RFC\s*\d+|ISO[\s/][\w.-]+|IEEE\s*[\w.-]+|MS-[A-Z]+|POSIX|SUSv\d|X\.509|"
    r"UEFI|ACPI|ELF|DWARF)\s*(?:'s)?[,;]?[ \t]*$"
)

# `§§10–23` is a RANGE, not a citation, and the second `§` would otherwise be
# read as one. Skipped rather than parsed: nothing in the corpus depends on
# checking a range endpoint, and mis-parsing one would be a false positive.
DOUBLE = re.compile(r"§§")

# The document a citation names, immediately before the `§`. Backticks,
# a possessive, `docs/` and a comma are all forms the corpus actually uses.
# **A generated HTML document is one of this project's own and is still not in
# the corpus**, which reads Markdown. `ADR-037` §7 cites `BRAND.html` §05, and
# before this the name did not match here, so the citation fell through to the
# bare-`§N` branch and was reported as a section of no document — a false
# positive about a section that exists. It is not `EXTERNAL` either: that is for
# somebody else's standard, and this is ours. Naming it makes `resolves()`
# return `None`, which is the path already built for "a document this checker
# does not hold" — counted, and not asserted about.
DOC_NAME = (
    r"(?:docs/)?`?"
    r"(?P<doc>ADR-\d{3}|SPEC-\d{2}|PLAN(?:-Archive-\d{2}[A-Za-z0-9.-]*)?\.md"
    r"|AGENTS\.md|README\.md|[A-Za-z][A-Za-z0-9.-]*\.html)"
    r"`?(?:'s|’s)?[,;]?[ \t]*"
)
EXPLICIT = re.compile(DOC_NAME + r"(?=§)")

# Placeholders. `§N`, `§X.Y` and `§8A.x` are the shapes this record uses when it
# means "some section"; `§TUI` is not a citation at all. They start with a
# non-digit or carry a non-numeric component, and both are excluded by CITE's
# own shape — kept here as the reason, not as a filter.

# A heading's title, with its number stripped — mirrors `index.py`'s `NUMBER`
# pattern. Used to detect a tombstone (`AGENTS.md` §2 rule 6): a numbered
# heading whose title, after the number, starts `Retired ` — e.g. `### 5.4
# Retired 2026-09-07: superseded by X. See `ADR-046` §3.` The section is not
# gone; every citation into it still resolves, and `retired-citations` is what
# tells a reader they landed on a marker rather than the content.
HEADING_REST = re.compile(r"^~*\**\s*§?\s*\d+[A-Z]?(?:\.\d+[A-Z]?)*\s*[.—:–-]?\s*(.*)$")


def heading_rest(body: str) -> str | None:
    m = HEADING_REST.match(body)
    return m.group(1).strip() if m else None


def is_tombstone(body: str) -> bool:
    rest = heading_rest(body)
    return bool(rest) and rest.startswith("Retired ")


def is_tombstone_line(line: str) -> bool:
    m = HEADING.match(line)
    return bool(m) and is_tombstone(m.group(2))


STRUCK = re.compile(r"~~.+?~~", re.S)


def mask_struck(text: str) -> str:
    """Blank out `~~struck~~` spans, preserving every offset and line break.

    **Struck text is superseded, not deleted** (`AGENTS.md` §2 rule 3), and the
    whole point of keeping it is that it records what was believed before. A
    checker that reported the stale count inside a strike would be reporting the
    record doing its job. `PLAN.md` §94.4 is the case that decides this: it
    struck five line-number citations *and named their replacements*, so
    checking the struck halves would turn a correct repair into a finding.

    Offsets are preserved rather than the span removed, so every line number
    this tool prints still points at the line a reader will open.
    """

    def blank(m: re.Match[str]) -> str:
        return "".join("\n" if c == "\n" else " " for c in m.group(0))

    return STRUCK.sub(blank, text)


def doc_key(path: Path) -> str:
    """The name a citation would use for this file.

    `ADR-015-Application-Packaging-and-Sandboxing.md` is cited as `ADR-015`;
    everything else is cited by filename.
    """
    m = re.match(r"^(ADR-\d{3}|SPEC-\d{2})-", path.name)
    return m.group(1) if m else path.name


# --------------------------------------------------------------------------
# Findings
# --------------------------------------------------------------------------


@dataclass
class Finding:
    check: str
    where: str  # file:line
    detail: str
    evidence: str = ""


@dataclass
class Report:
    findings: list[Finding] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def add(self, check: str, where: str, detail: str, evidence: str = "") -> None:
        self.findings.append(Finding(check, where, detail, evidence))

    def note(self, text: str) -> None:
        self.notes.append(text)


# --------------------------------------------------------------------------
# The corpus
# --------------------------------------------------------------------------


@dataclass
class Document:
    path: Path
    key: str
    lines: list[str]
    # number -> the line it is declared on. A number can repeat across a file
    # only by mistake, and that is itself reported.
    numbers: dict[str, int] = field(default_factory=dict)
    # 1-based line -> the innermost numbered section containing it.
    owner: list[str | None] = field(default_factory=list)
    # numbers whose heading is a tombstone (AGENTS.md §2 rule 6) — the section
    # still resolves, but a citation into it should say so.
    retired: set[str] = field(default_factory=set)


def load_documents(report: Report) -> dict[str, Document]:
    paths = sorted(DOCS.glob("*.md"))
    paths += [ROOT / n for n in EXTRA_DOCS if (ROOT / n).exists()]
    docs: dict[str, Document] = {}
    for path in paths:
        if path.name in ("INDEX.md",):
            continue  # generated by index.py; its own guard covers it
        lines = path.read_text(encoding="utf-8").splitlines()
        d = Document(path=path, key=doc_key(path), lines=lines)
        stack: list[tuple[int, str]] = []
        owner: list[str | None] = [None] * (len(lines) + 1)
        for i, line in enumerate(lines, start=1):
            m = HEADING.match(line)
            if m:
                level = len(m.group(1))
                nm = HEAD_NUM.match(m.group(2))
                while stack and stack[-1][0] >= level:
                    stack.pop()
                if nm:
                    number = nm.group(1)
                    d.numbers.setdefault(number, i)
                    stack.append((level, number))
                    if is_tombstone(m.group(2)):
                        d.retired.add(number)
            owner[i] = stack[-1][1] if stack else None
        d.owner = owner
        docs[d.key] = d
    if not docs:
        report.add("corpus", str(DOCS), "no documents found — the checker has nothing to check")
    return docs


def plan_family(docs: dict[str, Document]) -> set[str]:
    """Every number `PLAN.md §N` may legitimately resolve to.

    `PLAN.md` was split on 2026-08-29 and **nothing was renumbered** — `§88` is
    still `§88`, in `PLAN-Archive-06-2026-08-28.md`. `PLAN.md` stayed the
    citation target, so a `PLAN.md §N` citation resolves against the union.
    That is the pointer table at the top of `PLAN.md`, mechanised.
    """
    out: set[str] = set()
    for key, d in docs.items():
        if key == "PLAN.md" or key.startswith("PLAN-Archive-"):
            out |= set(d.numbers)
    return out


# --------------------------------------------------------------------------
# Check 1 — dangling section citations
# --------------------------------------------------------------------------


def unignored_files() -> set[str] | None:
    """Every file git would not ignore — tracked, or untracked and not ignored.

    **A gitignored file is one clone's scratch, not the repository**, ruled
    2026-09-12 (`.rulings/` R-158). `.launcher-pkg/` and `.qemu-endpoints/` are
    ignored scratch crates that cite `PLAN.md`, and reading them made the same
    commit measure one count in the main checkout and another in a clean
    worktree. An untracked file that is NOT ignored is still read, because it is
    about to be committed.

    `None` when git cannot answer (no git, or a worktree whose `.git` file
    points at a host path the container cannot see), and the caller then reads
    everything, as before this rule existed.
    """
    try:
        listed = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others",
             "--exclude-standard"],
            capture_output=True, check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return {entry.decode("utf-8", "replace") for entry in listed.split(b"\0") if entry}


def text_files() -> list[Path]:
    out: list[Path] = []
    unignored = unignored_files()
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(ROOT).as_posix()
        if any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
            continue
        if unignored is not None and relative not in unignored:
            continue
        # `vendor/` is foreign source citing a foreign specification — see
        # SKIP_PREFIXES. `VENDORING.md` is this project's own writing about it
        # and stays in.
        if any(relative.startswith(p) for p in SKIP_PREFIXES) and path.name != "VENDORING.md":
            continue
        if path.suffix in SOURCE_SUFFIXES or path.suffix == ".md":
            out.append(path)
    return sorted(out)


# Explicit citations this checker has read and found to be correct English
# rather than a broken reference. Every entry names the reason; an entry
# without one is a silenced finding, which is the failure mode an allowlist
# invites and the reason this list is kept short and in the open.
EXCEPTIONS: dict[tuple[str, str | None, str], str] = {
    (
        "docs/ADR-029-The-Update-Payload-Format.md",
        "ADR-018",
        "11",
    ): "hypothetical, not a reference — *'It is its own file rather than an "
    "`ADR-018` §11'* names a section that deliberately does not exist",
    **{
        ("docs/PLAN.md", None, n): "`PLAN.md` §94.4 QUOTES these five as the mistake it "
        "is correcting — they were line numbers written as section numbers. Reporting "
        "them would be reporting the record doing its job."
        for n in ("6054", "7315", "3304", "15094", "476")
    },
    # `PLAN.md` §102.4's standing register of tabled rulings has six entries
    # whose SUBJECT is a section that does not exist. An entry saying
    # *"`ADR-007` §3.5.1 does not exist and is cited 19 times"* has to spell it.
    # This is the cost of a checker whose findings are recorded in the corpus it
    # checks, and it is paid in the open rather than by loosening the check.
    **{
        ("docs/PLAN.md", doc, n): "`§102.4`'s register names the missing section — that "
        "is the entry's whole subject"
        for doc, n in (
            ("ADR-007", "3.5.1"),
            ("ADR-006", "6.6"),
            ("ADR-013", "7.3"),
            ("ADR-005", "7.9"),
            ("ADR-017", "3.1.13"),
            ("ADR-017", "8.1"),
            ("ADR-023", "10.2"),
        )
    },
    # The 2026-08-29 move left `PLAN.md` §95–§101 as pointer stubs with no
    # children, so every OLD subsection number is a section of no document. The
    # three places below are the ones that STATE that mapping — `PLAN.md`'s
    # correction table, the stubs themselves, and the two ADRs whose moved
    # sections declare *"bare `§N` in this body are `PLAN.md` sections"*. They
    # have to spell the old numbers to say what became of them.
    **{
        (rel, None, n): "a MAPPING statement — this line's job is to say what the old "
        "number became, so it has to spell the old number"
        for rel in (
            "docs/PLAN.md",
            "docs/PLAN-Archive-07-2026-08-29.md",
            "docs/ADR-029-The-Update-Payload-Format.md",
            "docs/ADR-012-The-Agent-as-a-First-Class-Operator.md",
            "docs/ADR-032-Asking-a-Running-Machine.md",
            "docs/ADR-005-Userland-Composition.md",
            "docs/ADR-017-Services-and-Supervision.md",
            "docs/ADR-006-Administration-and-Configuration-Surface.md",
        )
        for n in (
            [f"95.{i}" for i in range(1, 7)]
            + [f"96.{i}" for i in range(1, 8)]
            + ["97.2", "97.3"]
            + [f"98.{i}" for i in range(1, 5)]
            + [f"100.{i}" for i in range(1, 5)]
            + [f"101.{i}" for i in range(1, 6)]
        )
    },
}


def check_citations(docs: dict[str, Document], report: Report) -> None:
    family = plan_family(docs)
    everywhere: set[str] = set()
    for d in docs.values():
        everywhere |= set(d.numbers)

    def resolves(doc_name: str, number: str) -> bool | None:
        """True/False, or None when the target document is not in the corpus."""
        if doc_name == "PLAN.md" or doc_name.startswith("PLAN-Archive"):
            # `PLAN.md` stayed the citation target through the 2026-08-29
            # split and nothing was renumbered, so the family is one namespace.
            return number in family
        d = docs.get(doc_name)
        if d is None:
            return None
        return number in d.numbers

    def parent_note(doc_name: str, number: str) -> str:
        """Whether the citation's PARENT resolves, which is a different defect.

        `ADR-013` §7.3 means *§7, item 3* — a numbered list item, and this
        corpus does cite them. `ADR-010` §10.5 means nothing at all: `ADR-010`
        stops at §9. Both are unfollowable through `docs/index.json`, which is
        the discovery path `ADR-029` §16 ships to a cold reader, but only the
        second is certainly a broken reference — so they are told apart rather
        than lumped together and reported at one severity.
        """
        if "." not in number:
            return "and has no §" + number.split(".")[0] + " either"
        parent = number.rsplit(".", 1)[0]
        if resolves(doc_name, parent) is not True:
            return f"and has no §{parent} either — the whole path is broken"
        return (
            f"; §{parent} DOES exist, so this is either a list-item citation "
            f"(`§{parent} item {number.rsplit('.', 1)[1]}`) or a subsection that went away. "
            f"Either way `docs/index.json` cannot resolve it"
        )

    unresolvable = 0
    bare_total = 0
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("docs/INDEX.md") or rel.endswith("index.json") or rel == "docs/COMMANDS.md":
            continue
        # The checker's own docstring quotes citations as examples; excluding it
        # keeps a tool from being a source of the thing it checks for.
        if path.name == "claims.py":
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if "§" not in text:
            continue
        text = mask_struck(text)
        lines = text.splitlines()

        for lineno, line in enumerate(lines, start=1):
            if "§" not in line:
                continue
            explicit_at: dict[int, str] = {}
            for m in EXPLICIT.finditer(line):
                explicit_at[m.end()] = m.group("doc")
            doubles = {m.start() for m in DOUBLE.finditer(line)}
            for m in CITE.finditer(line):
                if m.start() in doubles or (m.start() - 1) in doubles:
                    continue
                number = m.group(1)
                where = f"{rel}:{lineno}"
                named = explicit_at.get(m.start())
                if named:
                    if EXCEPTIONS.get((rel, named, number)):
                        continue
                    verdict = resolves(named, number)
                    if verdict is None:
                        unresolvable += 1
                        continue
                    if not verdict:
                        report.add(
                            "citations",
                            where,
                            f"`{named}` has no §{number} {parent_note(named, number)}",
                            line.strip()[:160],
                        )
                    continue

                # A BARE `§N`. Which document it belongs to is genuinely
                # ambiguous — the corpus writes bare citations both for the
                # containing file and for a document named a paragraph earlier
                # — so this deliberately does NOT guess. It reports only the
                # numbers that are a section of **no document in the corpus**,
                # which is what a line number written as a section looks like
                # (`PLAN.md` §94.4 found five of those) and what a typo looks
                # like. Everything else is left alone; see the note below for
                # the size of the hole that leaves.
                bare_total += 1
                if EXTERNAL.search(line[: m.start()]):
                    continue
                if EXCEPTIONS.get((rel, None, number)):
                    continue
                if number not in everywhere:
                    report.add(
                        "citations",
                        where,
                        f"bare §{number} is not a section of ANY document in the corpus",
                        line.strip()[:160],
                    )
    report.note(
        f"citations: {unresolvable} citation(s) named a document not in the corpus and were "
        f"NOT checked."
    )
    report.note(
        f"citations: {bare_total} bare §N (no document named at the citation) were checked "
        f"only against the WHOLE corpus, so a bare §4 that means `ADR-017` §4 and lands in a "
        f"file whose own §4 is unrelated is NOT detected. Resolving a bare citation needs a "
        f"reader, and a checker that guessed would produce the confident-wrong output this "
        f"tool exists to find."
    )


# --------------------------------------------------------------------------
# Check 2 — paths asserted in prose
# --------------------------------------------------------------------------

# Only roots this project OWNS. `/proc`, `/sys` and `/dev` are the kernel's and
# a documented path there is a reading off a board, not a claim about this
# source tree — checking them would produce noise, not findings.
OWNED_ROOTS = ("/state", "/system", "/run/rivet", "/var/rivet")

DOC_PATH = re.compile(r"`(?P<path>/[A-Za-z0-9_./<>-]{2,})`")
# A mode stated near a path: `mode 700`, `mode 0700`, `mode 0755`.
MODE_NEAR = re.compile(r"mode\s+0?(?P<mode>[0-7]{3,4})\b")
# In Rust: a quoted absolute path and an octal literal on the same line.
CODE_PATH = re.compile(r'"(?P<path>/[A-Za-z0-9_./-]{2,})"')
CODE_MODE = re.compile(r"0o(?P<mode>[0-7]{3,4})\b")


def normalise(path: str) -> str:
    """`/state/app/<name>/data` -> a regex that matches a concrete instance."""
    body = re.escape(path.rstrip("/"))
    body = re.sub(r"\\<[A-Za-z0-9_ -]+\\>", "[^/]+", body)
    return body


def source_texts() -> list[tuple[str, str]]:
    out = []
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("docs/") or rel in EXTRA_DOCS:
            continue
        if path.suffix == ".md":
            continue
        try:
            out.append((rel, path.read_text(encoding="utf-8")))
        except (OSError, UnicodeDecodeError):
            continue
    return out


def check_paths(docs: dict[str, Document], report: Report) -> None:
    sources = source_texts()
    if not sources:
        report.add("paths", "tools/docs/claims.py", "no source files were read — refusing to "
                   "report every documented path as unbacked")
        return

    blob = "\n".join(t for _, t in sources)

    # 2a — a documented path under a root this project owns that appears
    # nowhere in the source. Conservative by construction: a path the code
    # composes from parts (`format!("{}/{DATA_DIR}", ...)`) will not match, so
    # this reports only paths that are wholly absent, and false negatives are
    # preferred to false positives.
    seen: set[tuple[str, str]] = set()
    unchecked = 0
    quoted = 0
    for key, d in docs.items():
        for i, line in enumerate(mask_struck("\n".join(d.lines)).splitlines(), start=1):
            # A blockquote is somebody else's words — usually the maintainer
            # proposing a name (`ADR-016` §1 has one, offered as a placeholder
            # for a better name; `.rulings/` R-035). A path inside one is a
            # suggestion, and reporting it as an unbacked claim would be
            # reporting a conversation.
            if line.lstrip().startswith(">"):
                quoted += 1
                continue
            for m in DOC_PATH.finditer(line):
                p = m.group("path").rstrip("/.,;:")
                if not p.startswith(OWNED_ROOTS):
                    unchecked += 1
                    continue
                if (key, p) in seen:
                    continue
                seen.add((key, p))
                # Accept if the path OR any ancestor of it down to two
                # components is spelled in the source. Deliberately generous:
                # the code composes most of these (`format!("{}/{DATA_DIR}",
                # app_dir(volume, name))`), and the archives record concrete
                # observations off a board — `/state/app/rivet-todo/data` is a
                # reading, not a claim about this tree. What survives is a
                # documented namespace the source has never heard of, which is
                # the only version of this check worth failing a build over.
                if re.search(normalise(p), blob):
                    continue
                parts = p.split("/")
                if any(
                    re.search(normalise("/".join(parts[: n + 1])), blob)
                    for n in range(len(parts) - 1, 1, -1)
                ):
                    continue
                report.add(
                    "paths",
                    f"{d.path.relative_to(ROOT).as_posix()}:{i}",
                    f"`{p}` is documented but appears in no source file",
                    line.strip()[:160],
                )

    # 2b — a mode claimed for a path, against the modes the code uses for that
    # exact path literal. This is the check that catches `ADR-015` §4.2's
    # *"`/state/app` is root-owned and mode 0700"*: the installer creates it
    # `0o755` and always has.
    code_modes: dict[str, set[str]] = {}
    code_where: dict[str, str] = {}
    for rel, text in sources:
        for i, line in enumerate(text.splitlines(), start=1):
            modes = {m.group("mode").lstrip("0") or "0" for m in CODE_MODE.finditer(line)}
            if not modes:
                continue
            for m in CODE_PATH.finditer(line):
                p = m.group("path").rstrip("/")
                if not p.startswith(OWNED_ROOTS):
                    continue
                code_modes.setdefault(p, set()).update(modes)
                code_where.setdefault(p, f"{rel}:{i}")

    for key, d in docs.items():
        for i, line in enumerate(mask_struck("\n".join(d.lines)).splitlines(), start=1):
            mm = MODE_NEAR.search(line)
            if not mm:
                continue
            claimed = mm.group("mode").lstrip("0") or "0"
            for pm in DOC_PATH.finditer(line):
                p = pm.group("path").rstrip("/.,;:")
                if p not in code_modes:
                    continue
                if claimed in code_modes[p]:
                    continue
                report.add(
                    "paths",
                    f"{d.path.relative_to(ROOT).as_posix()}:{i}",
                    f"`{p}` is documented as mode {mm.group('mode')}; the source uses "
                    + ", ".join(f"0o{m}" for m in sorted(code_modes[p]))
                    + f" ({code_where[p]})",
                    line.strip()[:160],
                )
    report.note(
        f"paths: {unchecked} documented path(s) outside {', '.join(OWNED_ROOTS)} were NOT "
        f"checked — `/proc`, `/sys` and `/dev` are the kernel's namespace and a path there is "
        f"a reading off a board, not a claim about this tree. {quoted} blockquote line(s) "
        f"were skipped for the same reason."
    )
    report.note(
        "paths: a documented path is accepted if the path OR any ancestor of it two "
        "components deep is spelled in the source, so `ADR-015` §3.2's `/state/app/<name>` "
        "passes even though the writable directory is `/state/app/<name>/data`. **That is "
        "the exact defect this whole exercise started from and this check does NOT catch "
        "it** — both paths exist, and only a reader can see that the sentence names the "
        "wrong one. What the machine caught was the MODE, in §4.2."
    )


# --------------------------------------------------------------------------
# Check 3 — counts that prose asserts and the tree can settle
# --------------------------------------------------------------------------

WORDS = {
    "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
    "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12,
    "thirteen": 13, "fourteen": 14, "fifteen": 15, "sixteen": 16,
    "seventeen": 17, "eighteen": 18, "nineteen": 19, "twenty": 20,
    "twenty-one": 21, "twenty-two": 22, "twenty-three": 23, "twenty-four": 24,
    "twenty-five": 25, "twenty-six": 26, "twenty-seven": 27, "twenty-eight": 28,
    "twenty-nine": 29, "thirty": 30,
    # **The table ran out on the day the count reached it, 2026-09-01**, and the
    # failure was worse than a wrong number: an unparseable word makes `hits`
    # stay zero, so the checker reports "the claim was not found — it moved, was
    # reworded, or was removed" about a claim sitting exactly where it always
    # was, correctly worded, and merely too large to read. A vocabulary that
    # stops at the current value guarantees that message on the next increment.
    "thirty-one": 31, "thirty-two": 32, "thirty-three": 33, "thirty-four": 34,
    "thirty-five": 35, "thirty-six": 36, "thirty-seven": 37, "thirty-eight": 38,
    "thirty-nine": 39, "forty": 40, "forty-one": 41, "forty-two": 42,
    "forty-three": 43, "forty-four": 44, "forty-five": 45, "forty-six": 46,
    "forty-seven": 47, "forty-eight": 48, "forty-nine": 49, "fifty": 50,
}


class Refused(Exception):
    """A derivation could not be made. It is a finding, never a fallback."""


def workspace_members() -> list[str]:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r"^members\s*=\s*\[(.*?)^\]", text, re.S | re.M)
    if not m:
        raise Refused("Cargo.toml has no `members = [ ... ]` block this parser recognises")
    names = re.findall(r'^\s*"([^"]+)"', m.group(1), re.M)
    if not names:
        raise Refused("Cargo.toml's `members` block parsed to nothing")
    return names


def workspace_excludes() -> list[str]:
    """The `[workspace] exclude` list — directories deliberately kept OUT.

    A crate listed here is a decision recorded in `Cargo.toml` beside its
    reason, not an orphan: `vendor/ixr-smb-server` declares its own workspace,
    and `cmd/rivet-displayd` links a shared-only library whose codegen flag
    cargo honours only outside a member. `tools/test-all.sh` walks nested
    workspaces, so an excluded crate is still built and tested — which is why
    the crates check must not report it as built by nothing.

    An absent `exclude` key is an empty list, not a refusal: most workspaces
    have none, and absence is the ordinary state of the key.
    """
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r"^exclude\s*=\s*\[(.*?)\]", text, re.S | re.M)
    if not m:
        return []
    return re.findall(r'"([^"]+)"', m.group(1))


def table_len(rel: str, name: str) -> int:
    """How many entries a `pub const NAME: &[Struct] = &[ ... ];` table holds.

    Counted by the struct literals that open at one indent inside the slice,
    which is how every such table in this workspace is written. **It refuses
    rather than guessing** if the constant is not there or the shape is not the
    one it knows — a count derived from a table this parser failed to find would
    be a confident wrong number, which is the thing being checked for.
    """
    path = ROOT / rel
    if not path.exists():
        raise Refused(f"{rel} does not exist")
    text = path.read_text(encoding="utf-8")
    m = re.search(rf"pub const {name}: &\[(\w+)\] = &\[\n(.*?)\n\];", text, re.S)
    if not m:
        raise Refused(f"no `pub const {name}: &[...] = &[ ... ];` in {rel}")
    entries = re.findall(rf"^    {m.group(1)} \{{", m.group(2), re.M)
    if not entries:
        raise Refused(f"`{name}` in {rel} parsed to no entries")
    return len(entries)


def _subject_const(module: str) -> str | None:
    path = ROOT / "rivet-core" / "src" / "operation" / f"{module}.rs"
    if not path.exists():
        return None
    m = re.search(r'pub const SUBJECT: &str = "([^"]+)"', path.read_text(encoding="utf-8"))
    return m.group(1) if m else None


def verb_subjects(verb: str) -> list[str]:
    """The subjects a verb takes, read off its `subject_table`.

    `rivet-core/src/operation/get.rs` says it itself: *"[`subject_table`] is the
    list, and this header deliberately no longer copies it"* — because the copy
    nobody maintains is the one that is wrong (`ADR-011`). This derivation reads
    the list rather than any prose about it.
    """
    path = ROOT / "rivet-core" / "src" / "operation" / f"{verb}.rs"
    if not path.exists():
        raise Refused(f"{path.relative_to(ROOT).as_posix()} does not exist")
    text = path.read_text(encoding="utf-8")
    m = re.search(r"fn subject_table\(\).*?\n\}", text, re.S)
    if not m:
        raise Refused(f"no `fn subject_table()` found in {verb}.rs")
    body = m.group(0)
    # `use super::file_diff as diff_subject;` — the alias exists because `diff`
    # is also the name of the module that computes one (get.rs's own comment).
    alias = dict(re.findall(r"use super::\{?(\w+) as (\w+)", text))
    names: list[str] = []
    for line in body.splitlines():
        nm = re.search(r"\bname:\s*(?P<expr>[A-Za-z_][A-Za-z0-9_:]*(?:::[A-Z_]+)?)", line)
        if not nm:
            continue
        expr = nm.group("expr")
        if "::" in expr:
            module, const = expr.rsplit("::", 1)
            module = {v: k for k, v in alias.items()}.get(module, module)
            if const == "SUBJECT":
                value = _subject_const(module)
            else:
                p = ROOT / "rivet-core" / "src" / "operation" / f"{module}.rs"
                value = None
                if p.exists():
                    cm = re.search(
                        rf'pub const {const}: &str = "([^"]+)"', p.read_text(encoding="utf-8")
                    )
                    value = cm.group(1) if cm else None
            if value is None:
                raise Refused(f"cannot resolve `{expr}` to a string — {verb}.rs")
            names.append(value)
        elif expr == "stop":
            # `Stop::ALL.iter().map(|stop| Subject { name: stop.as_str(), ...`
            machine = ROOT / "rivet-core" / "src" / "operation" / "machine.rs"
            am = re.search(
                r"pub const ALL: &'static \[Stop\] = &\[(.*?)\];",
                machine.read_text(encoding="utf-8"),
                re.S,
            )
            if not am:
                raise Refused("cannot find `Stop::ALL` in machine.rs")
            names += re.findall(r"Stop::(\w+)", am.group(1))
        else:
            raise Refused(f"unrecognised `name:` expression `{expr}` in {verb}.rs")
    if not names:
        raise Refused(f"`subject_table()` in {verb}.rs parsed to no subjects")
    return names


# Each entry: a claim's locator, and how the tree settles it. The locator is
# deliberately narrow — a regex over one document — so a rewording is reported
# as "the claim moved" rather than passing silently.
@dataclass
class Count:
    name: str
    doc: str
    patterns: list[re.Pattern[str]]
    derive: object  # () -> int
    what: str


COUNTS: list[Count] = [
    # **`AGENTS.md`'s crate count is GONE, and this checker is why.**
    #
    # It was the first Count here and the most productive: it caught the number
    # going stale seventeen times, three of them in one day, and twice caught
    # two agents correcting it in parallel. On 2026-09-07 the paragraph was
    # deleted rather than corrected an eighteenth time, under `AGENTS.md` §2
    # rule 7 and the count rule beside it — *never state a count of anything
    # this tree can be asked for.*
    #
    # **So the Count goes too, and its absence is the success.** A checker that
    # validates a sentence nobody writes any more is a fixture with no subject:
    # it fails as `the claim was not found where this checker looks for it`,
    # which is the right refusal for a claim that MOVED and the wrong one for a
    # claim that was deliberately retired. `Cargo.toml`'s `members` is the one
    # authority now and has no prose copy to disagree with it.
    #
    # The same thing happened to `PLAN.md`'s subject counts and the note below
    # says so; this is the second instance of the pattern and the first where
    # the prose was removed on purpose rather than archived.
    #
    # **If a document ever states a crate count again, restore a Count here
    # rather than trusting the sentence.**
    #

    # **`PLAN.md`'s prose copies of the subject counts are GONE, and that is
    # the outcome this checker exists to produce.**
    #
    # They lived in §102–§104 and went to `PLAN-Archive-08` with them on
    # 2026-09-02. Nothing was rewritten to remove them; archiving the sections
    # did it, and the right response is not to chase the sentences into a closed
    # record and keep validating history against today's build.
    #
    # `ADR-011` is the reason this is an improvement rather than a loss. The
    # fact now has ONE authority — `subject_table()` — and the two places that
    # restate it for a reader are both still checked below, in `get.rs`'s own
    # header and in `rivet --help`. A third copy in a build journal was the
    # second representation `ADR-011` names, and the count check is what kept
    # catching it.
    #
    # If a future `PLAN.md` states a subject count again, restore a Count here
    # rather than trusting the prose.

    Count(
        "get subjects, in get.rs's own header",
        "__source__:rivet-core/src/operation/get.rs",
        [re.compile(r"the\s+(?P<n>[a-z]+)\s+subjects this build reads")],
        lambda: len(verb_subjects("get")),
        "`subject_table()` in the same file",
    ),
    # `rivet --help` is the first prose a cold operator or agent reads, and its
    # subject list is a hand-maintained copy of `subject_table()` — the exact
    # second authority `get.rs`'s own header refuses to keep. It said `get`
    # reads TWO for as long as it read sixteen.
    Count(
        "get subjects, in `rivet --help`",
        "__source__:cmd/rivet/src/main.rs",
        [re.compile(r"`get` reads (?P<n>[A-Za-z]+) subjects", re.I)],
        lambda: len(verb_subjects("get")),
        "`rivet-core/src/operation/get.rs`'s `subject_table()`",
    ),
    # `ADR-006` §8.3 item 3 is a HARDWARE VERIFICATION CHECKLIST item — *"the
    # five paths `explain` describes are the five a running machine actually has
    # under `/etc`"* — so its number is load-bearing twice over. `/etc/motd`
    # arrived 2026-08-26 and made it six.
    Count(
        "/etc files the projection renders",
        "ADR-006",
        [re.compile(r"the (?P<n>[a-z]+) files the runtime `?/etc`? projection renders", re.I)],
        lambda: table_len("rivet-core/src/etc.rs", "ETC_FILES"),
        "`rivet_core::etc::ETC_FILES`",
    ),
    Count(
        "apply subjects, in `rivet --help`",
        "__source__:cmd/rivet/src/main.rs",
        # `[A-Za-z-]` and not `[A-Za-z]`: past twenty the word is hyphenated,
        # and `WORDS` spells it that way — the narrower class read TWENTY-ONE
        # as twenty.
        [re.compile(r"`apply` takes (?P<n>[A-Za-z-]+)", re.I)],
        lambda: len(verb_subjects("apply")),
        "`rivet-core/src/operation/apply.rs`'s `subject_table()`",
    ),
]


def check_counts(docs: dict[str, Document], report: Report) -> None:
    for c in COUNTS:
        try:
            actual = c.derive()  # type: ignore[operator]
        except Refused as exc:
            report.add(
                "counts",
                "tools/docs/claims.py",
                f"cannot derive {c.name} — {exc}",
                "a derivation that stopped working is a finding, never a pass",
            )
            continue
        except (OSError, re.error) as exc:  # pragma: no cover — defensive
            report.add("counts", "tools/docs/claims.py", f"cannot derive {c.name} — {exc}")
            continue

        if c.doc.startswith("__source__:"):
            rel = c.doc.split(":", 1)[1]
            p = ROOT / rel
            if not p.exists():
                report.add("counts", rel, f"claim site for {c.name} does not exist")
                continue
            lines = mask_struck(p.read_text(encoding="utf-8")).splitlines()
            where_name = rel
        else:
            # **Deliberately NOT the archive files.**
            #
            # Searching `PLAN-Archive-*.md` too was tried on 2026-09-02 and is
            # wrong: an archive is a CLOSED record, so its counts were true on
            # the day they were written and are not claims about this build.
            # Widening the search immediately reported `PLAN-Archive-07`'s
            # "answers eight subjects" and "answers thirteen subjects" as
            # defects, which would have meant editing history to satisfy a
            # checker.
            d = docs.get(c.doc)
            if d is None:
                report.add("counts", c.doc, f"claim site for {c.name} is not in the corpus")
                continue
            lines = mask_struck("\n".join(d.lines)).splitlines()
            where_name = d.path.relative_to(ROOT).as_posix()

        hits = 0
        for i, line in enumerate(lines, start=1):
            for pattern in c.patterns:
                for m in pattern.finditer(line):
                    word = m.group("n").lower()
                    if word in WORDS:
                        claimed = WORDS[word]
                    elif word.isdigit():
                        claimed = int(word)
                    else:
                        continue
                    hits += 1
                    if claimed != actual:
                        report.add(
                            "counts",
                            f"{where_name}:{i}",
                            f"{c.name}: the record says {word} ({claimed}); "
                            f"{c.what} says {actual}",
                            line.strip()[:160],
                        )
        if hits == 0:
            report.add(
                "counts",
                where_name,
                f"the claim about {c.name} was not found where this checker looks for it "
                f"— it moved, was reworded, or was removed",
                "; ".join(p.pattern for p in c.patterns),
            )


# --------------------------------------------------------------------------
# Check 4 — binaries named in prose that the workspace does not contain
# --------------------------------------------------------------------------

CMD_PATH = re.compile(r"`?\bcmd/(?P<name>[a-z][a-z0-9-]*)")

# Files whose whole subject is a name that no longer exists. `ADR-033` records
# the rename to Rivet and has to spell the old names to do it; the archives are
# a dated journal and were correct on the day they were written (`AGENTS.md`
# §2 rule 2 — the record shows its own history).
HISTORICAL = ("ADR-033", "PLAN-Archive-")

# A live document that names an old binary AS an old binary. Each entry says so;
# an entry without a reason is a silenced finding.
CRATE_EXCEPTIONS: dict[tuple[str, str], str] = {
    (
        "docs/ADR-027-Artifact-Naming.md",
        "rivet-update",
    ): "§9.3 IS the section that ruled the rename and has to name what was renamed "
    "— *'`cmd/rivet-update`, as it then was'*",
    (
        "docs/ADR-007-The-Configuration-Model.md",
        "rivet-update",
    ): "§7.2.4's 2026-08-29 correction says the paragraph was written when the binary "
    "still had that name, and points at `ADR-029` §14",
    (
        "AGENTS.md",
        "rivet-update",
    ): "§10's crate-count paragraph is a dated record of its own corrections and names "
    "the binary as it was called on 2026-08-24; the note beside it says the name is now "
    "stale and points at `ADR-029` §14",
    # **A `cmd/` path in ANOTHER repository**, which is a different kind of
    # exception from the four above and is marked as such rather than blended in
    # with them. `CMD_PATH` cannot tell whose tree a path belongs to, and Aura
    # is the maintainer's own Go/C++ project, in a repository of its own — it has
    # a `cmd/aura-controller` and this repository does not and should not.
    #
    # Both documents say which tree they mean in the sentence around the path.
    # The alternative was to spell the path less precisely so the pattern would
    # not match it, which would make the record worse to protect a checker.
    (
        "docs/SPEC-04-Aura-Cards.md",
        "aura-controller",
    ): "SPEC-04 is DERIVED FROM Aura, a separate repository, and "
    "its status block names that tree explicitly; `cmd/aura-controller` is Aura's binary "
    "and is not a Rivet crate",
    (
        "docs/PLAN.md",
        "aura-controller",
    ): "§109.3 cites Aura's own source to show a defect in it — the same separate "
    "repository `SPEC-04` is derived from",
    (
        "docs/PLAN.md",
        "tempmon-rivet",
    ): "TempMon is the maintainer's own separate project, in a repository of its own "
    "(§157), and `cmd/tempmon-rivet` is ITS binary — the same "
    "kind of cross-repository path as `aura-controller` above, and the surrounding "
    "sentence names TempMon as the actor",
    (
        "docs/PLAN.md",
        "tempmon",
    ): "TempMon's OTHER composition root, in the same separate repository — `cmd/tempmon` "
    "is the ordinary-PC binary and `cmd/tempmon-rivet` the Rivet one. §158.1 has to name "
    "it because the whole finding is that the machine holding the real data runs THAT "
    "root and therefore has no Rivet proxy in front of it",
    # **A binary that never existed, in the two places that SAY it never
    # existed.** These are not stale claims of presence; they are the record
    # doing its job. `AGENTS.md` §10 names `cmd/rivet-gitd` in order to correct
    # a struck count that had included it, and `ADR-026` §4's closing paragraph
    # is a retrospective — *"were all treated as hardware work for two days"* —
    # about what was believed on the day it was written.
    #
    # `ADR-026`'s OTHER mention was a real defect and was fixed on 2026-09-06
    # rather than excepted: it described the binary in the present tense, as
    # something this workspace has. The difference between the two is the whole
    # test for whether an entry belongs in this table.
    (
        "AGENTS.md",
        "rivet-gitd",
    ): "§10 names it to say there is no such directory — the sentence around it is the "
    "correction, and excepting it is what lets the check reach zero",
    # **`PLAN.md` §154 is the section that RECORDS this table**, so it has to
    # quote the entries. It went in on 2026-09-06 and immediately produced two
    # findings of its own, which is this check working rather than failing: a
    # section about names that do not exist necessarily names them.
    (
        "docs/PLAN.md",
        "rivet-gitd",
    ): "§154 records why this table has the entries it has, and quotes the sentence "
    "`ADR-026` §4 was corrected FROM — it cannot state the defect without naming it",
    (
        "docs/ADR-026-The-Development-and-Test-Loop.md",
        "rivet-gitd",
    ): "§4's closing paragraph is a retrospective about what was believed for two days, "
    "in the past tense. The present-tense mention earlier in the same section was a real "
    "claim of presence and was rewritten instead",
}


def check_crates(docs: dict[str, Document], report: Report) -> None:
    cmd_dir = ROOT / "cmd"
    if not cmd_dir.is_dir():
        report.add("crates", "cmd/", "there is no `cmd/` directory — refusing to report "
                   "every named binary as missing")
        return
    present = {p.name for p in cmd_dir.iterdir() if p.is_dir()}
    members = set()
    try:
        members = {m.rsplit("/", 1)[-1] for m in workspace_members()}
    except Refused as exc:
        report.add("crates", "Cargo.toml", f"cannot read the workspace members — {exc}")
    # A `cmd/` crate in `exclude` was kept out on purpose and with a reason
    # written beside it; it is a nested workspace that `tools/test-all.sh`
    # builds and tests. Only `cmd/` entries matter here, and only by name,
    # which is how `members` is compared too.
    excluded = {
        e.rsplit("/", 1)[-1] for e in workspace_excludes() if e.startswith("cmd/")
    }

    historical = 0
    for key, d in docs.items():
        if key.startswith(HISTORICAL):
            historical += 1
            continue
        for i, line in enumerate(mask_struck("\n".join(d.lines)).splitlines(), start=1):
            for m in CMD_PATH.finditer(line):
                name = m.group("name")
                if name in present:
                    continue
                rel_doc = d.path.relative_to(ROOT).as_posix()
                if CRATE_EXCEPTIONS.get((rel_doc, name)):
                    continue
                report.add(
                    "crates",
                    f"{d.path.relative_to(ROOT).as_posix()}:{i}",
                    f"`cmd/{name}` is named as though it exists; there is no such directory",
                    line.strip()[:160],
                )
    for name in sorted(present - members - excluded):
        report.add(
            "crates",
            "Cargo.toml",
            f"`cmd/{name}` is a directory but is not a workspace member — it is built by "
            f"nothing and tested by nothing",
        )
    report.note(
        f"crates: {historical} document(s) were NOT checked — `ADR-033` is ABOUT the rename "
        f"and has to spell the old names, and the `PLAN-Archive-*` files are a dated journal "
        f"that was correct on the day each entry was written."
    )


# --------------------------------------------------------------------------
# Dated media — the three checks below do not apply to them
# --------------------------------------------------------------------------

# **The line these three checks draw is DATE, not importance.** `PLAN.md` §159.3
# named the defect precisely: *perishable claims in an immortal medium*. A
# document that carries its own expiry is not the immortal medium, and checking
# it would produce findings nobody can act on:
#
# * `docs/PLAN.md` and `docs/PLAN-Archive-*.md` — a **dated journal**. An entry
#   says what was true on the day it was written and the date is on the entry.
# * `STATE.md` — declares itself *"what is true about Rivet right now"* and is
#   **rewritten wholesale on every suspend**, so its state claims are refreshed
#   by construction. §5 of it is literally a list of what is unproven.
# * `.rulings/RULINGS.md` — **the maintainer's own words**, which an agent does
#   not edit. History has to name what it changed; a check that fails on a new
#   ruling fails on a file its reader cannot fix, which is how a check becomes
#   the one nobody trusts.
DATED_MEDIA = (
    "docs/PLAN.md",
    "docs/PLAN-Archive-",
    "STATE.md",
    ".rulings/",
)

# **Generated, and therefore not writing.** `docs/INDEX.md` is `index.py`'s
# output and its lines are copies of headings from files that are checked where
# they are written. Leaving it in made the checks report a heading TWICE, and —
# worse — report one written in an exempt file, which is how this exemption was
# found: a `## §161 — … has never been there before` in `PLAN.md` cost a
# `rot-markers` finding one regeneration later, in a file no human types into.
#
# `docs/COMMANDS.md` joined 2026-09-13 for the identical reason, one layer
# down: it is `commands.py`'s output and its fenced blocks are copies of a
# BINARY'S OWN `--help` text, which is a Rust string literal already checked
# where it is written. A stale `PLAN.md §N` baked into `cargo-rivet`'s own
# `--help` (found this way, in `rivet-image`'s and `rivet-smbd`'s) is a defect
# in that string, already counted from the `.rs` file; reproducing it here
# made the SAME defect spread to a second location on every regeneration,
# which is exactly the false positive this exemption exists to prevent.
GENERATED = ("docs/INDEX.md", "docs/index.json", "docs/COMMANDS.md")


def dated(rel: str) -> bool:
    """Outside all three of the checks below — see DATED_MEDIA and GENERATED."""
    return any(rel.startswith(p) or rel == p for p in DATED_MEDIA + GENERATED)


# --------------------------------------------------------------------------
# Check 5 — `PLAN.md` is uncitable from outside itself
# --------------------------------------------------------------------------

# Ruled 2026-09-06 (`PLAN.md` §159.6 rule 1). **A `PLAN.md §N` written anywhere
# but `PLAN.md` is a finding**, whatever it resolves to today.
#
# The measurement is the argument. `ADR-0NN` is cited 8,648 times and 0.9% of
# those are dead; `PLAN.md` is cited 1,029 times and ~97% are dead. That is not
# bad luck — ADR numbers are immortal by rule, one decision per file, never
# renumbered, while `PLAN.md` is a journal that gets appended to, cut and
# renumbered. **A citation into it points at the one document exempt from the
# stability every other citation relies on.**
#
# So this check does not ask whether the target resolves. `check_citations`
# already does that and will keep doing it for the journal family. This one
# says the pointer should not have been written: **a fact worth citing is worth
# promoting to an ADR.**
PLAN_CITE = re.compile(
    r"`?(?:docs/)?(?P<doc>PLAN(?:-Archive-\d{2}[A-Za-z0-9.-]*)?\.md)`?"
    r"(?:'s|’s)?[,;]?[ \t]*(?=§)"
)

PLAN_CITE_DETAIL = (
    "`PLAN.md` is cited from outside itself — it is a JOURNAL, renumbered and cut, "
    "and ~97% of citations into it are already dead. Promote the fact to an ADR and "
    "cite that instead"
)


def check_plan_citations(docs: dict[str, Document], report: Report) -> None:
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if dated(rel) or path.name == "claims.py":
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if "§" not in text:
            continue
        for lineno, line in enumerate(mask_struck(text).splitlines(), start=1):
            for m in PLAN_CITE.finditer(line):
                report.add(
                    "plan-citations",
                    f"{rel}:{lineno}",
                    PLAN_CITE_DETAIL,
                    line.strip()[:160],
                )


# --------------------------------------------------------------------------
# Check 6 — perishable phrasing in an undated medium
# --------------------------------------------------------------------------

# Ruled 2026-09-06 (`PLAN.md` §159.6 rule 2). These are the phrases that assert
# **state** rather than **reason**, counted in §159.3 and found to number ~1,205.
#
# **The distinction that matters:** *"Chrome does not accept Ed25519
# certificates"* will be true in ten years. *"Nothing on the card starts it"*
# was false in a fortnight. The corpus treats them identically and only one of
# them is a decision. A state claim belongs in a **test** or a **generated
# artifact**; in prose it is a cache with no invalidation.
#
# `not yet` and `does not yet` are the purest of them: the phrase's own word
# concedes it expires and nothing is watching the clock.
ROT_MARKERS = (
    "is the only",
    "no such",
    "does not exist",
    "has never",
    "does not yet",
    "not yet",
    "is not built",
    "not implemented",
    "nothing starts",
)
ROT_RE = re.compile("|".join(re.escape(p) for p in ROT_MARKERS), re.I)

# A phrase inside backticks is being MENTIONED, not used. `AGENTS.md` §2 rule 5
# has to list `is the only`, `has never` and `not yet` in order to say they
# perish, and a check that fired on the sentence stating its own rule would be
# the first thing anyone switched off.
CODE_SPAN = re.compile(r"`[^`]*`")

# A Rust doc comment. §159.1 measured 771,873 words of these against 528,135 in
# tracked Markdown — **more of this project's prose is in the code**, so a check
# that read only `docs/` would miss the larger half of its own subject.
DOC_COMMENT = re.compile(r"^\s*//[/!]")


def check_rot_markers(docs: dict[str, Document], report: Report) -> None:
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if dated(rel) or path.name == "claims.py":
            continue
        is_md = path.suffix == ".md"
        if not is_md and path.suffix != ".rs":
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for lineno, line in enumerate(mask_struck(text).splitlines(), start=1):
            if not is_md and not DOC_COMMENT.match(line):
                continue
            spans = [(m.start(), m.end()) for m in CODE_SPAN.finditer(line)]
            for m in ROT_RE.finditer(line):
                if any(a <= m.start() and m.end() <= b for a, b in spans):
                    continue
                phrase = m.group(0).lower()
                report.add(
                    "rot-markers",
                    f"{rel}:{lineno}",
                    f'"{phrase}" asserts STATE, not reason — a claim like this was true '
                    f"when typed and carries no expiry. Put it in a test or a generated "
                    f"artifact, or rewrite it as the reason that will still hold",
                    line.strip()[:160],
                )


# --------------------------------------------------------------------------
# Check 7 — prose that outlived the thing it describes (WARNING, not a gate)
# --------------------------------------------------------------------------

# Ruled 2026-09-06 (`PLAN.md` §159.6 rule 3): **removing a feature removes its
# prose, in the same commit.** §159.4 is the cost of not doing it — the SSH
# purge was 563 references across 37 files for a feature that had not functioned
# in months, and *"deleting the code was minutes; deleting the story about the
# code was the afternoon."*
#
# **This one is a WARNING and does not gate**, by ruling, because it is the one
# with real false-positive risk: a document may name a thing precisely because
# it is describing its removal, and a design section may name something proposed
# and never built. Both are correct writing. It earns a gate by being trusted
# first, not by being switched on hard.
GHOST_NAME = re.compile(r"`(?P<name>(?:rivet|rivetos)-[a-z][a-z0-9-]*)`")


def live_identifiers() -> set[str]:
    """Every `rivet-*` / `rivetos-*` name the TREE actually contains.

    Read from the filesystem and from `Cargo.toml`, never from prose — a
    checker that accepted a name because a comment mentioned it would confirm
    exactly the thing it is looking for.
    """
    live: set[str] = set()
    for parent in (ROOT, ROOT / "cmd", ROOT / "examples", ROOT / "vendor", ROOT / "tools"):
        if not parent.is_dir():
            continue
        for child in parent.iterdir():
            if child.is_dir():
                live.add(child.name)
                # Directories are `rivet-*` and packages are `rivetos-*`; the
                # two spellings of one crate are both live.
                if child.name.startswith("rivet-"):
                    live.add("rivetos-" + child.name[len("rivet-"):])
    for toml in list(ROOT.glob("*/Cargo.toml")) + list(ROOT.glob("*/*/Cargo.toml")):
        try:
            text = toml.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for m in re.finditer(r'^\s*name\s*=\s*"([^"]+)"', text, re.M):
            live.add(m.group(1))
    # **Not everything named `rivet-something` is a crate.** `rivet-system` and
    # `rivet-app` are the two signing keys (`keys/`); `rivet-br`, `rivet-card-cargo`
    # and their siblings are the Docker volumes the container build lives in.
    # Both sets are read from the tree rather than written into an exceptions
    # list, because both are real things whose names would otherwise be reported
    # as ghosts for ever.
    #
    # The container scripts are harvested wholesale because they are **executable
    # code, not prose**: a name a script spells is a name something uses.
    for pub in (ROOT / "keys").glob("*.pub"):
        live.add(pub.stem)
    for script in (ROOT / "tools" / "container").glob("*.sh"):
        try:
            text = script.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        live |= set(re.findall(r"\brivet-[a-z0-9-]*", text))
    return live


# A live document that names a dead or never-built thing ON PURPOSE. Every
# entry says why; an entry without a reason is a silenced finding.
#
# **This list existed before the check was switched on**, by ruling: a warning
# whose first run is mostly noise is the warning nobody reads. What is
# deliberately NOT in it is `rivet-gitd` — see the note in `check_ghosts`.
GHOST_EXCEPTIONS: dict[tuple[str, str], str] = {
    # The 2026-08-24 rename. These three documents record what PID 1 and the
    # supervisor were CALLED, which cannot be written without spelling the old
    # names — the same reason `ADR-033` is skipped whole.
    **{
        (rel, name): "the 2026-08-24 rename is this passage's subject; it has to spell "
        "the name it is saying went away"
        for rel in (
            "docs/ADR-005-Userland-Composition.md",
            "docs/ADR-014-Workspace-and-Crate-Layout.md",
            "docs/ADR-017-Services-and-Supervision.md",
        )
        for name in ("rivet-init", "rivet-supervisor", "rivet-supervi")
    },
    (
        "docs/ADR-014-Workspace-and-Crate-Layout.md",
        "rivet-api",
    ): "the sentence is *'No `rivet-api` crate yet'* — it names the crate in order to say "
    "it does not exist, which is the opposite of the defect this check looks for",
    (
        "docs/ADR-014-Workspace-and-Crate-Layout.md",
        "rivet-adapter-",
    ): "a PREFIX, not a name — *'the `rivet-adapter-` prefix continues to mean…'*",
    **{
        (
            "docs/ADR-017-Services-and-Supervision.md",
            "rivet-apid",
        ): "the boot-stage table marks it `later`; the column IS the statement that it "
        "is unbuilt",
        (
            "docs/ADR-027-Artifact-Naming.md",
            "rivet-install",
        ): "§9.3 records the name that was RECOMMENDED and lost — a rejected candidate, "
        "which the decision cannot be read without",
    },
    **{
        (
            "docs/design-candidates/Bluetooth.md",
            "rivet-br-bt",
        ): "the throwaway Docker volume for the Bluetooth sandbox, named only in the "
        "candidate that proposes it; no script has ever spelled it",
    },
    # **The five that survived the 2026-09-07 sweep, and they are the good kind.**
    #
    # This check's first run found ten sites describing `rivet-gitd` in the
    # PRESENT tense across six ADRs, for a crate purged 2026-08-30 when
    # `ADR-035` §4 moved git to smart HTTP inside `rivet-mcpd`. Five were
    # rewritten to name what actually serves git. These five name the crate ON
    # PURPOSE, as the retired thing, and each says so in its own sentence — the
    # decision that moved it cannot be recorded without naming what moved.
    **{
        (rel, "rivet-gitd"): "names the crate as RETIRED, in a passage whose subject is "
        "the SSH-era arrangement or the decision that ended it — the sentence says so "
        "itself (purged 2026-08-30; `ADR-035` §4)"
        for rel in (
            "docs/ADR-009-Reuse-Posture.md",
            "docs/ADR-032-Asking-a-Running-Machine.md",
            "docs/ADR-035-The-API-First-Control-Plane.md",
        )
    },
}


def check_ghosts(docs: dict[str, Document], report: Report) -> None:
    """Names in prose that the tree does not contain.

    **`rivet-gitd` is the first run's whole justification and is NOT excepted.**
    Six ADRs name it — `ADR-029` says *"The board already runs `rivet-gitd`"*,
    `ADR-035` describes *"the programs `rivet-gitd` already execs"*, `ADR-034`
    lists it beside `rivet-smbd`, which is real — and `git ls-files`, `cmd/` and
    `Cargo.toml`'s members agree that no such crate has ever existed here. That
    is §159.4's defect exactly: the description surface outliving the
    implementation surface, in the present tense, in documents a cold agent is
    told to trust.
    """
    live = live_identifiers()
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if dated(rel) or path.name == "claims.py":
            continue
        if path.suffix != ".md" or rel.startswith("docs/ADR-033"):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for lineno, line in enumerate(mask_struck(text).splitlines(), start=1):
            for m in GHOST_NAME.finditer(line):
                name = m.group("name")
                if name in live or GHOST_EXCEPTIONS.get((rel, name)):
                    continue
                report.add(
                    "ghosts",
                    f"{rel}:{lineno}",
                    f"`{name}` is named in prose and the tree contains no such crate, "
                    f"directory or package — either it was removed and its story was not, "
                    f"or it was never built and the paragraph reads as though it was",
                    line.strip()[:160],
                )


# --------------------------------------------------------------------------
# Check 8 — narrative in the immortal layer (WARNING, not a gate)
# --------------------------------------------------------------------------

# Ruled 2026-09-07 (`.rulings/` R-153; `AGENTS.md` §2 rule 7). **The body of an
# ADR or SPEC, and every doc comment, is undated and tells no story** — the
# decision, the contract and the reason, never when a sentence was added or
# what happened on a board that afternoon. That narrative belongs in the
# commit message, in `PLAN.md` as a dated entry, or in a dated `## History`
# section, which is why both are excluded below.
#
# **This is a WARNING and MUST NOT become a gate by this tool's own choice.**
# It counts dates, and a date is a coarse proxy for a story — the cheapest way
# to satisfy a date-counter is to delete a date from a sentence that needed
# one. Rule 7 says so explicitly: *"It becomes a ratchet only by a later
# ruling, once there is evidence about what it actually flags."*
NARRATIVE_DATE = re.compile(r"\b2026-\d\d-\d\d\b")

HISTORY_HEADING = re.compile(r"^~*\**\s*History\b", re.I)

NARRATIVE_ADR_SPEC_DETAIL = (
    "a dated line in an ADR/SPEC body — the immortal layer is undated and tells "
    "no story (AGENTS.md §2 rule 7); the narrative belongs in the commit, in "
    "PLAN.md, or in a dated ## History entry"
)
NARRATIVE_RS_DATE_DETAIL = (
    "a dated `///`/`//!` line — a doc comment states the decision and the reason, "
    "never when it changed (AGENTS.md §2 rule 7)"
)
NARRATIVE_RS_QUOTE_DETAIL = (
    "a `///`/`//!` line quotes a design document with `>` — cite the number and "
    "state the code's own reason instead of reproducing the source (AGENTS.md §2 "
    "rule 7)"
)


def history_and_status_ranges(lines: list[str]) -> tuple[int, set[int]]:
    """The first `---` line (end of the status block), and every line inside a
    `## History` section.

    Every ADR/SPEC opens with a status block that ends at the file's first
    `---` (`AGENTS.md` §2's header convention) — a date there is metadata, not
    narrative. **No `---` found** means the convention was not followed and
    nothing is exempted as a status block, rather than guessing a boundary.
    """
    status_end = 0
    for i, line in enumerate(lines, start=1):
        if line.strip() == "---":
            status_end = i
            break
    in_history: set[int] = set()
    in_section = False
    for i, line in enumerate(lines, start=1):
        m = HEADING.match(line)
        if m:
            level = len(m.group(1))
            if level == 2 and HISTORY_HEADING.match(m.group(2).strip()):
                in_section = True
            elif level <= 2:
                in_section = False
        if in_section:
            in_history.add(i)
    return status_end, in_history


def check_narrative(docs: dict[str, Document], report: Report) -> None:
    scanned_docs = 0
    for key, d in docs.items():
        if not (key.startswith("ADR-") or key.startswith("SPEC-")):
            continue
        scanned_docs += 1
        rel = d.path.relative_to(ROOT).as_posix()
        status_end, in_history = history_and_status_ranges(d.lines)
        masked = mask_struck("\n".join(d.lines)).splitlines()
        for i, line in enumerate(masked, start=1):
            if i <= status_end or i in in_history:
                continue
            if is_tombstone_line(d.lines[i - 1]):
                continue
            # **A HEADING is never a finding here, because rule 6 forbids the
            # repair.** Found 2026-09-07 by sampling seventy of this check's own
            # findings: seven of the seventy were headings like
            # `## 6. Ruled, 2026-08-22: private now, sanitized public mirror
            # later`, and flagging one asks a writer to rewrite a heading that
            # `AGENTS.md` §2 rule 6 says is immortal. **A check that demands
            # something the contract forbids is a check people learn to ignore**,
            # and the two rules landed on the same day without anyone noticing
            # they disagreed.
            #
            # The date in a heading is also the one place it is least harmful:
            # a heading is read as a label, not as a live instruction, and
            # `index.py` already surfaces it as structure rather than prose.
            if HEADING.match(d.lines[i - 1]):
                continue
            spans = [(m.start(), m.end()) for m in CODE_SPAN.finditer(line)]
            for m in NARRATIVE_DATE.finditer(line):
                if any(a <= m.start() and m.end() <= b for a, b in spans):
                    continue
                report.add(
                    "narrative",
                    f"{rel}:{i}",
                    NARRATIVE_ADR_SPEC_DETAIL,
                    line.strip()[:160],
                )
                break  # one finding per line, however many dates it carries

    scanned_rs = 0
    for path in text_files():
        if path.suffix != ".rs":
            continue
        scanned_rs += 1
        rel = path.relative_to(ROOT).as_posix()
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for i, line in enumerate(mask_struck(text).splitlines(), start=1):
            m = DOC_COMMENT.match(line)
            if not m:
                continue
            spans = [(s.start(), s.end()) for s in CODE_SPAN.finditer(line)]
            dated = False
            for dm in NARRATIVE_DATE.finditer(line):
                if not any(a <= dm.start() and dm.end() <= b for a, b in spans):
                    dated = True
                    break
            if dated:
                report.add("narrative", f"{rel}:{i}", NARRATIVE_RS_DATE_DETAIL, line.strip()[:160])
                continue
            body = line[m.end():]
            if body.lstrip().startswith(">"):
                report.add(
                    "narrative", f"{rel}:{i}", NARRATIVE_RS_QUOTE_DETAIL, line.strip()[:160]
                )
    report.note(
        f"narrative: {scanned_docs} ADR/SPEC document(s) and {scanned_rs} .rs file(s) were "
        f"scanned. PLAN.md and its archives, STATE.md, .rulings/, docs/design-candidates/ and "
        f"docs/INDEX.md are dated media and are not this check's subject — they are excluded "
        f"by not being an ADR or SPEC in the first place."
    )


# --------------------------------------------------------------------------
# Check 9 — a deep citation written in Rust (a ratchet from the day it lands)
# --------------------------------------------------------------------------

# Ruled 2026-09-07 (`.rulings/` R-153; `AGENTS.md` §2 rule 8). **Code cites the
# document; prose cites the section.** A citation in `.rs` names an ADR or SPEC
# and at most its H2 — `ADR-017 §4`, never `ADR-017 §4.2`. A dotted number
# belongs in Markdown, where a tool can rewrite it and rule 6's tombstones
# catch what it misses.
DEEP_CITATION_DETAIL_TEMPLATE = (
    "a DOTTED citation in Rust — code cites at most an H2 (AGENTS.md §2 rule 8); "
    "move the deeper reference into the surrounding prose or cite the H2 alone"
)


def check_deep_citations(docs: dict[str, Document], report: Report) -> None:
    scanned = 0
    for path in text_files():
        if path.suffix != ".rs":
            continue
        scanned += 1
        rel = path.relative_to(ROOT).as_posix()
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if "§" not in text:
            continue
        for i, line in enumerate(mask_struck(text).splitlines(), start=1):
            if "§" not in line:
                continue
            explicit_at: dict[int, str] = {}
            for m in EXPLICIT.finditer(line):
                explicit_at[m.end()] = m.group("doc")
            doubles = {m.start() for m in DOUBLE.finditer(line)}
            for m in CITE.finditer(line):
                if m.start() in doubles or (m.start() - 1) in doubles:
                    continue
                named = explicit_at.get(m.start())
                # Rule 8 is about an ADR or SPEC specifically — "a citation
                # written in .rs names an ADR or SPEC and at most its H2".
                # PLAN.md is already forbidden from Rust outright by rule 5's
                # `plan-citations` check, and AGENTS.md/README.md are not the
                # documents rule 8 is protecting from a deep restructure.
                if not named or not re.match(r"^(ADR-\d{3}|SPEC-\d{2})$", named):
                    continue
                if "." not in m.group(1):
                    continue
                report.add(
                    "deep-citations",
                    f"{rel}:{i}",
                    DEEP_CITATION_DETAIL_TEMPLATE,
                    line.strip()[:160],
                )
    report.note(
        f"deep-citations: {scanned} .rs file(s) scanned for a dotted ADR/SPEC citation only — "
        f"a dotted PLAN.md, AGENTS.md or README.md citation is not this check's subject (the "
        f"first is already forbidden outright by plan-citations). vendor/ and target/ are "
        f"excluded by text_files(), and claims.py itself is not a .rs file, so none of the "
        f"three needs its own exception here. A BARE `§N.M` in Rust (no document named at the "
        f"citation) is not counted — see `citations`' own note on why a bare number is not "
        f"resolved."
    )


# --------------------------------------------------------------------------
# Check 10 — a citation that resolves to a tombstone (WARNING, not a gate)
# --------------------------------------------------------------------------

# A tombstone (check 2a / `AGENTS.md` §2 rule 6) is not a dead citation —
# `check_citations` already resolves it, because the heading is still there.
# It is a citation that reaches a MARKER rather than the content that used to
# be at that number, and a reader following it should be told rather than
# discovering it by reading the section they landed on.
RETIRED_CITATION_DETAIL_TEMPLATE = (
    "resolves to a RETIRED section — the citation is not broken, but it points at a "
    "tombstone rather than the content that used to be there"
)


def check_retired_citations(docs: dict[str, Document], report: Report) -> None:
    plan_retired: set[str] = set()
    for key, d in docs.items():
        if key == "PLAN.md" or key.startswith("PLAN-Archive-"):
            plan_retired |= d.retired

    checked = 0
    for path in text_files():
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("docs/INDEX.md") or rel.endswith("index.json") or rel == "docs/COMMANDS.md":
            continue
        if path.name == "claims.py":
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if "§" not in text:
            continue
        checked += 1
        for lineno, line in enumerate(mask_struck(text).splitlines(), start=1):
            if "§" not in line:
                continue
            explicit_at: dict[int, str] = {}
            for m in EXPLICIT.finditer(line):
                explicit_at[m.end()] = m.group("doc")
            doubles = {m.start() for m in DOUBLE.finditer(line)}
            for m in CITE.finditer(line):
                if m.start() in doubles or (m.start() - 1) in doubles:
                    continue
                named = explicit_at.get(m.start())
                if not named:
                    continue
                number = m.group(1)
                if named == "PLAN.md" or named.startswith("PLAN-Archive"):
                    retired_here = plan_retired
                else:
                    target = docs.get(named)
                    retired_here = target.retired if target else set()
                if number in retired_here:
                    report.add(
                        "retired-citations",
                        f"{rel}:{lineno}",
                        f"`{named}` " + RETIRED_CITATION_DETAIL_TEMPLATE,
                        line.strip()[:160],
                    )
    report.note(
        f"retired-citations: {checked} file(s) with a `§` were checked, and only NAMED "
        f"citations (`ADR-NNN §X.Y`) — a bare `§N` is ambiguous about which document it means, "
        f"same as `citations`' own bare handling, so it is not checked against retirement."
    )


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------

CHECKS = {
    "citations": check_citations,
    "paths": check_paths,
    "counts": check_counts,
    "crates": check_crates,
    "plan-citations": check_plan_citations,
    "rot-markers": check_rot_markers,
    "ghosts": check_ghosts,
    "narrative": check_narrative,
    "deep-citations": check_deep_citations,
    "retired-citations": check_retired_citations,
}

# **Ratchets, not gates** (`PLAN.md` §159.7). These two are baselined at a
# four-figure backlog that nobody is going to clear, and the ruling was explicit
# that clearing it was never the plan: the check fails when a number **goes up**,
# and a number that falls is good news printed as good news. The clippy baseline
# went 287 → 126 → 90 → 85 with no cleanup sprint, because people touching a
# file for other reasons cleaned up as they passed; this will fall the same way.
#
# The four original checks keep the stricter rule — a fixed finding that stays
# in the baseline fails, because there are thirty-odd of them and a stale entry
# among thirty is a second representation of the record's state. Among a
# thousand it is just noise.
#
# **`citations` joined them on 2026-09-06, and that was NOT part of the ruling.**
# The evidence for it is that `--check` was **red on `main`** when these three
# were added — 378 new and 11 newly fixed — because the sweep repairs citations
# faster than anyone regenerates a 357-row baseline, and *"FIXED, now edit this
# file"* on every commit is how a guard stops being run. It is loosened on the
# **decrease** side only; a citation that breaks still fails on the commit that
# breaks it. Raise it with the maintainer rather than treating it as settled.
#
# **`deep-citations` joined on 2026-09-07 (`.rulings/` R-153) and is different
# in kind from the other three: it is a ratchet from day one rather than one
# that started as a gate and was loosened.** `AGENTS.md` §2 rule 8 calls it
# "a gate from the day it lands" — there is no judgement in a dotted number —
# and the ratchet mechanics are used here only so today's backlog does not
# have to be cleared in the same change that adds the check.
RATCHET_ONLY = {"plan-citations", "rot-markers", "citations", "deep-citations"}

# Reported, never gated — see checks 7, 8 and 10. Excluded from the baseline
# too: a warning pinned to a number is a gate wearing a different hat.
#
# **`narrative` MUST stay here.** `AGENTS.md` §2 rule 7 is explicit that it
# becomes a ratchet only by a later ruling, because it counts dates and a date
# is a coarse proxy for a story — gating it rewards deleting a date from a
# sentence that needed one.
WARN_ONLY = {"ghosts", "narrative", "retired-citations"}


def main() -> int:
    argv = sys.argv[1:]
    check_mode = "--check" in argv
    as_json = "--json" in argv
    writing = "--write-baseline" in argv
    only = None
    if "--only" in argv:
        only = argv[argv.index("--only") + 1].split(",")

    report = Report()
    docs = load_documents(report)
    for name, fn in CHECKS.items():
        if only and name not in only:
            continue
        fn(docs, report)

    if writing:
        if only:
            print("--write-baseline needs the whole corpus; drop --only.", file=sys.stderr)
            return 2
        return write_baseline(report)

    if as_json:
        print(
            json.dumps(
                {
                    "findings": [f.__dict__ for f in report.findings],
                    "not_checked": report.notes,
                },
                indent=2,
                ensure_ascii=False,
            )
        )
    else:
        by_check: dict[str, list[Finding]] = {}
        for f in report.findings:
            by_check.setdefault(f.check, []).append(f)
        for name in CHECKS:
            if only and name not in only:
                print(f"\n== {name} — NOT RUN (--only {','.join(only)})")
                continue
            hits = by_check.get(name, [])
            label = ""
            if name in WARN_ONLY:
                label = " — WARNING, not gated"
            elif name in RATCHET_ONLY:
                label = " — ratchet: fails only if this number goes UP"
            print(f"\n== {name} — {len(hits)} finding(s){label}")
            for f in hits:
                print(f"  {f.where}")
                print(f"      {f.detail}")
                if f.evidence:
                    print(f"      | {f.evidence}")
        print("\n== what was NOT checked")
        for n in report.notes:
            print(f"  {n}")
        print(f"\n{len(report.findings)} finding(s) total")

    if not check_mode:
        return 0
    if only:
        print(
            "\n--check with --only is a partial run and is not a gate; "
            "the baseline covers the whole corpus.",
            file=sys.stderr,
        )
        return 1 if report.findings else 0
    return gate(report)


BASELINE = Path(__file__).resolve().parent / "claims-baseline.txt"


def key(f: Finding) -> str:
    """A finding's identity, with the line number deliberately left out.

    A baseline keyed by `file:line` would go stale on the next paragraph
    inserted above it, and a guard that cries wolf gets switched off — which is
    `AGENTS.md` §10.5's clippy complaint and the reason `index.py --check` had
    to stop reporting a moved commit hash as drift. So the key is the check and
    the finding's own sentence, and the COUNT is what is pinned.
    """
    return f"{f.check}\t{f.detail}"


def load_baseline() -> dict[str, int]:
    if not BASELINE.exists():
        return {}
    out: dict[str, int] = {}
    for line in BASELINE.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        count, _, rest = line.partition("\t")
        out[rest] = int(count)
    return out


def gate(report: Report) -> int:
    """`--check`'s verdict, against the findings the register has already tabled.

    **Why a baseline and not a bare "any finding fails".** Thirty-two of these
    are in `PLAN.md` §102.4's standing register because the *repair* is a
    maintainer's ruling, not because nobody has looked. A guard that fails for
    them for ever is a guard nobody can put in a CI job, and one nobody runs
    catches nothing — which is exactly how the sentence in `ADR-015` §3.2
    survived five days.

    **Three verdicts, and the third is the one that matters.** A finding not in
    the baseline fails: it is new. A finding whose count EXCEEDS the baseline
    fails: it spread. **A baseline entry that no longer fires also fails**, and
    that is deliberate — a stale baseline is a second representation of the
    record's state that nobody maintains, which is the `ADR-011` problem this
    tool exists to catch, one level up. Fixing a finding is expected to require
    editing this file in the same change.
    """
    counts = gated_counts(report)
    base = load_baseline()

    new = {k: n for k, n in counts.items() if k not in base}
    spread = {k: (n, base[k]) for k, n in counts.items() if k in base and n > base[k]}
    stale = {k: n for k, n in base.items() if counts.get(k, 0) < n}

    # A ratchet's backlog is allowed to fall silently-but-noisily: reported as
    # good news, and not a failure. See RATCHET_ONLY for why these two differ
    # from the four that came first.
    fell = {k: n for k, n in stale.items() if k.partition("\t")[0] in RATCHET_ONLY}
    for k in fell:
        del stale[k]
    for k, was in sorted(fell.items()):
        check, _, detail = k.partition("\t")
        print(
            f"\nFELL {was} → {counts.get(k, 0)} [{check}] {detail}\n"
            f"       — good, and not a failure. Re-baseline when convenient: "
            f"`uv run tools/docs/claims.py --write-baseline`."
        )

    if not (new or spread or stale):
        print(
            f"\nOK — {sum(counts.values())} gated finding(s), all of them pinned in "
            f"{BASELINE.name}; nothing new and nothing spread."
        )
        return 0

    for k, n in sorted(new.items()):
        check, _, detail = k.partition("\t")
        print(f"\nNEW ({n}×) [{check}] {detail}", file=sys.stderr)
    for k, (n, was) in sorted(spread.items()):
        check, _, detail = k.partition("\t")
        print(f"\nSPREAD {was} → {n} [{check}] {detail}", file=sys.stderr)
    for k, was in sorted(stale.items()):
        check, _, detail = k.partition("\t")
        now = counts.get(k, 0)
        print(
            f"\nFIXED {was} → {now} [{check}] {detail}\n"
            f"       — good. Update {BASELINE.name} in this change; a baseline "
            f"nobody maintains is the defect this tool is for.",
            file=sys.stderr,
        )
    print(
        f"\nFAIL — {len(new)} new, {len(spread)} spread, {len(stale)} newly fixed. "
        f"Regenerate with `uv run tools/docs/claims.py --write-baseline` once the "
        f"remaining findings are the ones you mean to leave.",
        file=sys.stderr,
    )
    return 1


def gated_counts(report: Report) -> dict[str, int]:
    """Findings the baseline pins, keyed and counted. Warnings are not among them."""
    counts: dict[str, int] = {}
    for f in report.findings:
        if f.check in WARN_ONLY:
            continue
        counts[key(f)] = counts.get(key(f), 0) + 1
    return counts


def write_baseline(report: Report) -> int:
    counts = gated_counts(report)
    lines = [
        "# Findings `tools/docs/claims.py --check` accepts, with their counts.",
        "#",
        "# NOT a suppression list. Every one of these is tabled in `PLAN.md` §102.4",
        "# because the REPAIR is a maintainer's ruling — the finding itself is not in",
        "# doubt. `--check` fails on anything new, on anything that spread, AND on any",
        "# entry here that no longer fires, so this file cannot quietly rot.",
        "#",
        "# Regenerate: uv run tools/docs/claims.py --write-baseline",
        "",
    ]
    lines += [f"{n}\t{k}" for k, n in sorted(counts.items())]
    BASELINE.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {BASELINE.name} — {len(counts)} distinct finding(s), "
          f"{sum(counts.values())} occurrence(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
