# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write `docs/COMMANDS.md` from the binaries' own `--help` — plan §9.1.

**Why generated rather than hand-written.** This project's retired operating
manual carried a manifest-key table by hand: `cargo rivet --help` listed ten
keys and the document had to say, in prose, that the canonical order was
sixteen. A hand-written command reference is the same defect waiting to happen at a
larger scale — every verb, every flag, of every binary this project ships,
kept in sync with `--help` by a human remembering to. `tools/docs/index.py`'s
own argument applies unchanged: a second representation of something that
lives elsewhere is `ADR-011`'s problem, and the cheap guard is to regenerate
and compare.

**Why it shells out to the container rather than running binaries directly.**
Most of what this generates is Linux software — `rivet-mcpd`, `rivet-netd`,
`rivet-smbd` and the rest use raw Linux syscalls that do not link on Windows,
which is this project's own development host (`AGENTS.md` §10.5). The four
host-side binaries (`cargo-rivet`, `rivet-fleet`, `rivet-provision`,
`rivet-reset`) build natively on Windows too, but running every entry the
SAME way — through `tools/container/exec.sh`, against a binary built by
`cargo build --workspace --bins` in that same container — means one code path
answers for all of them, and a host/board split cannot quietly drift into two
different `--help` outputs for the same source.

**Which binaries, and which are left out.** `tools/docs/commands-manifest.tsv`
is the one list both this file and `tools/docs/check-commands.sh` read, so the
generator (run by hand, from a workstation with `uv` and `docker`) and the
in-container gate (`tools/test-all.sh`, which has neither) agree about what is
generated and why something is not. Six binaries are not: one is cross-only
and cannot link for the container's own architecture, one is PID 1 and never
returns from `main`, and four refuse before they would ever print a usage
line — a hardware dependency, a supervisor-only environment variable, or a
calling convention that is not `--help` at all. Each reason is in the
manifest, not just here.

    uv run tools/docs/commands.py           # regenerate docs/COMMANDS.md
    uv run tools/docs/commands.py --check   # fail if it would change

**What `tools/test-all.sh` runs is a DIFFERENT script, on purpose.** The build
container has neither `uv` nor a Python interpreter — checked 2026-09-13,
`exec: "uv": executable file not found` and the same for `python3` — so a
script that needs either cannot be the thing that gates a container run.
`tools/docs/check-commands.sh` is POSIX `sh`, reads the same manifest, and
diffs the SAME binaries' `--help` against `docs/COMMANDS.md` using only tools
already in that image. It does not regenerate the file; regenerating is this
file's job, run from a workstation, same as `index.py` and `claims.py` always
have been.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# `§` and em dashes are all over what the binaries print. A Windows console
# defaults to cp1252, which would mangle every one of them silently —
# `claims.py` carries the identical guard for the identical reason.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except (AttributeError, OSError):  # pragma: no cover — not a real console
        pass

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tools" / "docs" / "commands-manifest.tsv"
OUT = ROOT / "docs" / "COMMANDS.md"

# `exec.sh` reads `RIVET_DOCKER` itself, out of the environment this process
# inherits — nothing here needs to know its value to pass it along.
EXEC_SH = ROOT / "tools" / "container" / "exec.sh"


def load_manifest() -> list[dict]:
    rows: list[dict] = []
    for line in MANIFEST.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        name, kind, include, note = (line.split("\t", 3) + ["", "", "", ""])[:4]
        rows.append(
            {
                "name": name.strip(),
                "kind": kind.strip(),
                "include": include.strip() == "yes",
                "note": note.strip(),
            }
        )
    if not rows:
        print(f"commands.py: {MANIFEST} named no binaries at all — refusing to "
              f"write an empty reference", file=sys.stderr)
        sys.exit(1)
    return rows


def help_text(name: str) -> tuple[str, int]:
    """Run `<name> --help` inside the build container; return (text, exit code).

    stdout and stderr are combined by the shell that runs INSIDE the
    container (`2>&1`), not by this process — a binary that writes to both
    (several here do: a JSON log line to stderr, then the message to stdout)
    has one true interleaving, and only redirecting at the source preserves
    it. `capture_output` on two separate pipes would not.
    """
    # `sh` runs `exec.sh` explicitly rather than relying on the shebang, which
    # Windows' own process creation does not honour — this generator is meant
    # to run from the Windows workstation that is this project's development
    # host (`AGENTS.md` §10.5), same as `index.py` and `claims.py` already do.
    cmd = ["sh", str(EXEC_SH), "sh", "-c", f"/target/debug/{name} --help 2>&1; echo EXIT:$?"]
    try:
        # **`encoding="utf-8"` is not decoration.** The corpus is full of `§`
        # and em dashes (`claims.py`'s own note on the same hazard), and a
        # Windows console defaults `text=True` to cp1252 — every one of those
        # bytes would decode WRONG rather than fail loudly, which is worse:
        # a mangled character reads as a typo, not as a bug in this tool.
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=60,
            encoding="utf-8", errors="strict",
        )
    except subprocess.TimeoutExpired:
        print(f"commands.py: {name} --help did not return in 60s — is it in the "
              f"manifest by mistake? (see commands-manifest.tsv's PID-1 row for "
              f"what that looks like)", file=sys.stderr)
        sys.exit(1)
    out = proc.stdout
    if proc.returncode != 0:
        print(f"commands.py: tools/container/exec.sh itself failed for {name} "
              f"(exit {proc.returncode}) — is the image built and is `cargo build "
              f"--workspace --bins` current? stderr:\n{proc.stderr}", file=sys.stderr)
        sys.exit(1)
    m = re.search(r"EXIT:(-?\d+)\s*$", out)
    if not m:
        print(f"commands.py: could not read {name}'s exit code out of its own "
              f"output — the container invocation is not doing what this file "
              f"assumes", file=sys.stderr)
        sys.exit(1)
    exit_code = int(m.group(1))
    text = out[: m.start()]
    # The container's own banner ("linked worktree — using build cache ...")
    # is exec.sh's, not the binary's, and only appears on the FIRST line of
    # STDERR of the container process — never inside a binary's own output,
    # since it is printed by exec.sh before the binary runs at all. It does
    # not appear here because it is not part of the redirected `2>&1` inside
    # the container's `sh -c`; it goes to the docker client's own stderr on
    # the host, which `proc.stderr` above already discarded once the exit
    # code check passed.
    return normalise_version(text.rstrip("\n")), exit_code


# **A version line's commit is replaced by a placeholder**, found 2026-09-13.
# `cargo-rivet --help` opens with `cargo-rivet 0.1.86 (<git describe>)`, so the
# reference changed on every commit: it was generated in a worktree whose
# container could not resolve the commit ("an unknown commit") and then failed
# in the main checkout, where it could. The version number stays; only the
# parenthesised build identity, which says which checkout ran the tool, goes.
# `check-commands.sh` applies the same substitution, and the two must agree.
VERSION_LINE = re.compile(r"^(\S+ \d+\.\d+\.\d+\S*) \([^)]*\)$", re.MULTILINE)


def normalise_version(text: str) -> str:
    return VERSION_LINE.sub(r"\1 (<commit>)", text)


def render(rows: list[dict]) -> str:
    included = [r for r in rows if r["include"]]
    excluded = [r for r in rows if not r["include"]]
    host = [r for r in included if r["kind"] == "host"]
    board = [r for r in included if r["kind"] == "board"]

    out = [
        "# `docs/` — command reference",
        "",
        "* **Status:** Generated. Do not edit by hand.",
        "* **Generated by:** `uv run tools/docs/commands.py`",
        "* **Guard:** `uv run tools/docs/commands.py --check` (from a workstation "
        "with `uv` and `docker`) and `tools/docs/check-commands.sh` (inside the "
        "build container, wired into `tools/test-all.sh`) both fail if this is "
        "stale.",
        "",
        "---",
        "",
        "## What this is, and where each entry comes from",
        "",
        f"**Every binary below is run as `<name> --help` inside the build "
        f"container** (`tools/container/exec.sh`), against whatever "
        f"`cargo build --workspace --bins` produced there. The text after each "
        f"heading is copied verbatim — stdout and stderr interleaved exactly as "
        f"the binary wrote them, nothing added and nothing summarised.",
        "",
        f"**{len(included)} of {len(rows)} binaries are generated here.** The "
        f"rest have no usable `--help` surface off a running machine or off this "
        f"project's own build — see the table at the foot of this document for "
        f"which, and why. `tools/docs/commands-manifest.tsv` is the list both "
        f"this generator and the in-container check read, so the two cannot name "
        f"different binaries.",
        "",
        "---",
        "",
        "## Host-side binaries",
        "",
        "Installed onto a workstation — `cargo install --path cmd/<name> --force --locked` "
        "from this repository's root, once per binary, per `AGENTS.md` §10. "
        "**`cargo-rivet` and `rivet-provision` are installed together**: "
        "`cargo rivet provision` runs the second one as its interview, needs no "
        "source checkout once it is there, and refuses a pair whose builds "
        "disagree (`.rulings/` R-191, R-192).",
        "",
    ]
    for r in host:
        out += render_entry(r)
    out += [
        "---",
        "",
        "## Board-side binaries",
        "",
        "Ship on the card. `--help` is checked here, inside the build container, "
        "against the same source `cargo build --workspace --bins` compiles for "
        "every other check in this repository — it is not run against a board "
        "and does not stand in for one (`AGENTS.md` §10.5's own rule: *\"it works "
        "in QEMU\" is not \"it works\", and neither is \"it compiles\"*).",
        "",
    ]
    for r in board:
        out += render_entry(r)

    out += [
        "---",
        "",
        "## Not generated",
        "",
        "| Binary | Why |",
        "| :-- | :-- |",
    ]
    for r in excluded:
        note = r["note"].replace("|", "\\|")
        out.append(f"| `{r['name']}` | {note} |")
    out.append("")
    return "\n".join(out) + "\n"


def render_entry(row: dict) -> list[str]:
    text, exit_code = help_text(row["name"])
    status = "" if exit_code == 0 else f" — exits {exit_code} against `--help`"
    lines = [f"### `{row['name']}`{status}", "", "```"]
    lines += text.splitlines() if text else [""]
    lines += ["```", ""]
    return lines


def main() -> int:
    rows = load_manifest()
    md = render(rows)
    if "--check" in sys.argv:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else None
        if current != md:
            print("STALE, regenerate with `uv run tools/docs/commands.py`:", file=sys.stderr)
            print(f"  {OUT}", file=sys.stderr)
            return 1
        included = sum(1 for r in rows if r["include"])
        print(f"docs/COMMANDS.md is current — {included} binaries generated, "
              f"{len(rows) - included} not")
        return 0
    # `newline="\n"`: on Windows `write_text` would otherwise write CRLF, and
    # the in-container check compares against the LF bytes `.gitattributes`
    # now pins for this file.
    OUT.write_text(md, encoding="utf-8", newline="\n")
    included = sum(1 for r in rows if r["include"])
    print(f"wrote {OUT} — {included} binaries generated, {len(rows) - included} not")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
