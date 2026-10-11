# /// script
# requires-python = ">=3.12"
# ///
"""Cut the public tree: copy one committed revision of this repository into a
clone of the public repository, without what stays private, and check what was
written before anyone commits it.

`ADR-010` §6.5 rules what the public repository carries. Going public copies
the tree and never the history, so what this writes is exactly what is
published. It stages the result and stops: committing, tagging and pushing are
done by hand, after a person has looked.

    uv run tools/release/cut.py --into DIR [--rev REV] [--repository URL]

DIR is a git working tree outside this repository — a clone of the public
repository, or a fresh `git init`. Everything in it except `.git` is replaced.

What it leaves out, and why, is `PRIVATE` below. What it checks afterwards:

* no private path was written;
* none of the maintainer's own values appears in any byte of any file, read
  from `tools/release/forbidden.txt` — private, beside this script, and never
  copied, because the list of values must not publish them;
* no absolute workstation path appears;
* the generated references are regenerated from the published tree alone, and
  `claims.py`'s gating checks find nothing in it;
* every Cargo workspace in the copy resolves with its own lockfile, so nothing
  depends on a file that was left behind.

A failed check exits non-zero and leaves DIR as written, unstaged, so the
finding can be read where it is.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
from collections import Counter
from pathlib import Path

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

ROOT = Path(__file__).resolve().parents[2]
FORBIDDEN = ROOT / "tools" / "release" / "forbidden.txt"

# The public repository's address, written into the workspace's `repository`.
DEFAULT_REPOSITORY = "https://github.com/caladil/Rivet"
SOURCE_REPOSITORY_LINE = 'repository = "https://github.com/caladil/Rivet-Dev"'

# What is never copied, as (pattern, reason). A pattern ending in `/` is a
# directory; one containing `*` is matched against the whole path, where `*`
# does not cross a `/`; anything else is one exact path.
PRIVATE: list[tuple[str, str]] = [
    ("STATE.md", "the present state of the maintainer's own machines (ADR-010 §6.5)"),
    ("AGENTS.md", "the maintainer's brief for agents in the private repository (ADR-010 §6.5)"),
    (".rulings/", "the maintainer's own words (ADR-010 §6.5)"),
    ("docs/PLAN.md", "the working journal (ADR-010 §6.5)"),
    ("docs/PLAN-*.md", "the working journal's archives and plans (ADR-010 §6.5)"),
    (".github/", "CI runs on the maintainer's own runner and is not carried"),
    ("keys/*.pub", "a card built from the public tree must trust its builder's keys, not the maintainer's"),
    ("tools/docs/rulings.py", "the guard on the rulings, which are not carried"),
    ("tools/release/forbidden.txt", "the maintainer's own values, which the cut checks for"),
    # Generated from every document, private ones included, so regenerated
    # below from the published tree alone rather than copied.
    ("docs/INDEX.md", "regenerated from the published tree"),
    ("docs/index.json", "regenerated from the published tree"),
    ("tools/docs/claims-baseline.txt", "regenerated from the published tree"),
]
REGENERATED = {"docs/INDEX.md", "docs/index.json", "tools/docs/claims-baseline.txt"}

# `docs/RELEASE-BLOCKERS.md` §7.2's re-check, as a pattern: a Windows drive's
# Dev or Users directory, or a projects directory under a home-relative dev.
# The bracket keeps this line from matching itself.
ABSOLUTE_PATH = re.compile(rb"[A-Za-z]:[\\/](Dev|Users)[\\/](Projects|Scratchpad)|~/dev/project[s]", re.I)


class Refused(Exception):
    """A reason to stop, stated for the person reading it."""


def private(path: str) -> str | None:
    """Why `path` is not published, or None when it is."""
    for pattern, reason in PRIVATE:
        if pattern.endswith("/"):
            if path.startswith(pattern):
                return reason
        elif "*" in pattern:
            if fnmatch.fnmatchcase(path, pattern) and path.count("/") == pattern.count("/"):
                return reason
        elif path == pattern:
            return reason
    return None


def git(*args: str, cwd: Path = ROOT) -> str:
    done = subprocess.run(
        ["git", "-C", str(cwd), *args], capture_output=True, text=True, encoding="utf-8"
    )
    if done.returncode != 0:
        raise Refused(f"git {' '.join(args)} failed in {cwd}:\n{done.stderr.strip()}")
    return done.stdout.strip()


def prepare(into: Path) -> None:
    """Make `into` an empty working tree of a repository that is not this one."""
    if into == ROOT or ROOT in into.parents:
        raise Refused(f"{into} is inside this repository; the cut goes somewhere else.")
    if not (into / ".git").is_dir():
        raise Refused(
            f"{into} is not a git clone. Clone the public repository there, or "
            f"`git init -b main` it, and run this again — the cut replaces a working "
            f"tree and will not delete a directory it cannot identify."
        )
    common = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=into))
    if common.resolve() == (ROOT / ".git").resolve():
        raise Refused(f"{into} is a worktree of this repository, not a separate one.")

    def writable(func, path, _exc):  # Windows marks git's own objects read-only
        os.chmod(path, stat.S_IWRITE)
        func(path)

    for entry in into.iterdir():
        if entry.name == ".git":
            continue
        if entry.is_dir() and not entry.is_symlink():
            shutil.rmtree(entry, onexc=writable)
        else:
            entry.unlink()


def export(rev: str, into: Path) -> tuple[int, Counter[str], list[str]]:
    """Write `rev`'s tree into `into`, minus PRIVATE.

    `core.autocrlf=false`, so each file is written as the bytes git stores and
    not as this workstation would check it out.
    """
    proc = subprocess.Popen(
        ["git", "-C", str(ROOT), "-c", "core.autocrlf=false", "archive", "--format=tar", rev],
        stdout=subprocess.PIPE,
    )
    assert proc.stdout is not None
    written = 0
    left_out: Counter[str] = Counter()
    executable: list[str] = []
    with tarfile.open(fileobj=proc.stdout, mode="r|") as tar:
        for member in tar:
            if member.isdir() or member.name == "pax_global_header":
                continue
            reason = private(member.name)
            if reason:
                left_out[reason] += 1
                continue
            if not member.isfile():
                raise Refused(f"{member.name} is not a regular file, and the cut copies only those.")
            source = tar.extractfile(member)
            assert source is not None
            target = into / member.name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read())
            if member.mode & 0o111:
                executable.append(member.name)
            written += 1
    if proc.wait() != 0:
        raise Refused(f"git archive {rev} failed.")
    return written, left_out, executable


def rewrite_repository(into: Path, url: str) -> None:
    manifest = into / "Cargo.toml"
    text = manifest.read_bytes().decode("utf-8")
    found = text.count(SOURCE_REPOSITORY_LINE)
    if found != 1:
        raise Refused(
            f"the workspace's repository line was found {found} time(s), not once:\n"
            f"  {SOURCE_REPOSITORY_LINE}\nThe cut will not guess which to rewrite."
        )
    manifest.write_bytes(text.replace(SOURCE_REPOSITORY_LINE, f'repository = "{url}"').encode("utf-8"))


def run_tool(into: Path, script: str, *args: str) -> subprocess.CompletedProcess[str]:
    """Run one of the PUBLISHED tree's own tools, against that tree."""
    env = {k: v for k, v in os.environ.items() if k != "RIVET_DOCS_ROOT"}
    env["PYTHONIOENCODING"] = "utf-8"
    return subprocess.run(
        [sys.executable, str(into / script), *args],
        cwd=into,
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=env,
    )


def regenerate(into: Path) -> None:
    for script, args in (("tools/docs/index.py", ()), ("tools/docs/claims.py", ("--write-baseline",))):
        done = run_tool(into, script, *args)
        if done.returncode != 0:
            raise Refused(f"{script} {' '.join(args)} failed in the published tree:\n{done.stdout}{done.stderr}")


def published_files(into: Path):
    for path in into.rglob("*"):
        if ".git" in path.relative_to(into).parts or not path.is_file():
            continue
        yield path, path.relative_to(into).as_posix()


def forbidden_patterns() -> list[bytes]:
    """The maintainer's own values, lowercased, from the private list."""
    if not FORBIDDEN.is_file():
        raise Refused(
            f"{FORBIDDEN.relative_to(ROOT)} is missing, so the maintainer's own values "
            f"cannot be checked for. The cut does not publish an unchecked tree."
        )
    return [
        line.strip().lower().encode("utf-8")
        for line in FORBIDDEN.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def check_identity(into: Path) -> str:
    """The identity the clone will commit and tag with, refused if it is private.

    A commit's author, its committer and an annotated tag's tagger each carry an
    e-mail address that a public repository shows to everyone, and a clone
    inherits the workstation's own identity unless it is told otherwise. So the
    clone's effective identities — configuration and environment both, as
    `git var` resolves them — are checked against the same private list as the
    tree. The tagger is the committer identity.
    """
    patterns = forbidden_patterns()
    idents = {role: git("var", f"GIT_{role}_IDENT", cwd=into) for role in ("AUTHOR", "COMMITTER")}
    for role, ident in idents.items():
        # `Name <email> 1760000000 +0000`: the timestamp is not part of the identity.
        who = ident.rsplit(" ", 2)[0]
        if any(pattern in who.lower().encode("utf-8") for pattern in patterns):
            raise Refused(
                f"{into} would {role.lower()} as {who!r}, which forbidden.txt lists as "
                f"private, and a public repository shows every commit's address. Set "
                f"the clone's own identity first, for example:\n"
                f"  git -C {into} config user.email <id>+<login>@users.noreply.github.com"
            )
    return idents["COMMITTER"].rsplit(" ", 2)[0]


def verify(into: Path) -> list[str]:
    """Every finding against the written tree; empty when it is clean."""
    findings: list[str] = []
    patterns = forbidden_patterns()

    for path, rel in published_files(into):
        reason = private(rel)
        if reason and rel not in REGENERATED:
            findings.append(f"{rel}: written although it is private — {reason}")
        data = path.read_bytes()
        lowered = data.lower()
        for number, pattern in enumerate(patterns, start=1):
            at = lowered.find(pattern)
            if at >= 0:
                line = data.count(b"\n", 0, at) + 1
                findings.append(f"{rel}:{line}: carries forbidden.txt pattern {number}")
        for match in ABSOLUTE_PATH.finditer(data):
            line = data.count(b"\n", 0, match.start()) + 1
            findings.append(f"{rel}:{line}: an absolute workstation path")

    for rel in sorted(REGENERATED):
        if not (into / rel).is_file():
            findings.append(f"{rel}: was not regenerated")

    claims = run_tool(into, "tools/docs/claims.py", "--only", "paths,counts,crates")
    if claims.returncode != 0:
        findings.append(f"claims.py's gating checks failed in the published tree:\n{claims.stdout}{claims.stderr}")

    for lock in sorted(into.rglob("Cargo.lock")):
        if ".git" in lock.relative_to(into).parts:
            continue
        manifest = lock.parent / "Cargo.toml"
        done = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(manifest)],
            cwd=lock.parent,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        if done.returncode != 0:
            findings.append(
                f"{manifest.relative_to(into).as_posix()}: does not resolve from the published tree:\n"
                f"{done.stderr.strip()}"
            )
    return findings


def stage(into: Path, executable: list[str]) -> None:
    """Stage the tree, with each executable file's mode set in the index.

    On Windows a working tree carries no executable bit, so a commit made from
    it would publish every script as 0644 and break it for everyone else.

    The regenerated documents are written by this platform's tools, which on
    Windows end their lines in CRLF; the development repository normalizes them
    on the way in, and the clone must too whatever its own `core.autocrlf` says,
    or the published index differs from the development one on every line.
    `--renormalize` is the second pass because git declines to convert a file
    whose staged copy already carries CRLF, which is what an earlier cut into
    the same clone leaves behind.
    """
    git("-c", "core.autocrlf=input", "add", "-A", cwd=into)
    git("-c", "core.autocrlf=input", "add", "--renormalize", ".", cwd=into)
    for start in range(0, len(executable), 100):
        git("update-index", "--chmod=+x", "--", *executable[start : start + 100], cwd=into)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--into", required=True, type=Path, help="a git clone of the public repository")
    parser.add_argument("--rev", default="HEAD", help="the revision of this repository to cut (default HEAD)")
    parser.add_argument("--repository", default=DEFAULT_REPOSITORY, help="the public repository's URL")
    args = parser.parse_args()
    into = args.into.resolve()

    try:
        commit = git("rev-parse", "--verify", f"{args.rev}^{{commit}}")
        described = git("describe", "--tags", "--always", commit)
        prepare(into)
        identity = check_identity(into)
        written, left_out, executable = export(commit, into)
        rewrite_repository(into, args.repository)
        regenerate(into)
        findings = verify(into)
    except Refused as why:
        print(f"cut.py: REFUSED. {why}", file=sys.stderr)
        return 1

    print(f"cut     {commit[:12]} ({described}) into {into}")
    print(f"wrote   {written} file(s), {len(executable)} of them executable")
    for reason, count in sorted(left_out.items()):
        print(f"left    {count:4d}  {reason}")
    print(f"as      {identity}")
    print(f"set     repository = \"{args.repository}\"")
    print("made    docs/INDEX.md, docs/index.json, tools/docs/claims-baseline.txt from the published tree")

    if findings:
        print(f"\ncut.py: {len(findings)} finding(s). Nothing was staged.", file=sys.stderr)
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        return 1

    try:
        stage(into, executable)
    except Refused as why:
        print(f"cut.py: the tree is clean but staging it failed. {why}", file=sys.stderr)
        return 1
    print("checked no private path, no forbidden value, no absolute path; claims gates clean; every workspace resolves")
    print(f"staged  everything, with {len(executable)} executable mode(s) set in the index")
    print(f"\nNothing is committed. Review it, then commit in {into}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
