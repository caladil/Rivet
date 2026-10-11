# Draw a deterministic, stratified sample of `narrative` findings with context,
# so a classification pass has something to read rather than a count.
import json
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1])
N_PER = int(sys.argv[2]) if len(sys.argv) > 2 else 35

raw = subprocess.run(
    ["uv", "run", "tools/docs/claims.py", "--only", "narrative", "--json"],
    cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
).stdout
findings = [f for f in json.loads(raw)["findings"] if f["check"] == "narrative"]

md = [f for f in findings if f["where"].rsplit(":", 1)[0].endswith(".md")]
rs = [f for f in findings if f["where"].rsplit(":", 1)[0].endswith(".rs")]

rng = random.Random(20260907)  # deterministic: the same sample twice
picked = rng.sample(md, min(N_PER, len(md))) + rng.sample(rs, min(N_PER, len(rs)))

blocks = []
cache: dict[str, list[str]] = {}
for n, f in enumerate(picked, 1):
    path, lineno = f["where"].rsplit(":", 1)
    lineno = int(lineno)
    if path not in cache:
        cache[path] = (ROOT / path).read_text(encoding="utf-8").splitlines()
    lines = cache[path]
    lo, hi = max(0, lineno - 6), min(len(lines), lineno + 5)
    ctx = []
    for i in range(lo, hi):
        mark = ">>" if i + 1 == lineno else "  "
        ctx.append(f"{mark} {lines[i]}")
    blocks.append(f"### {n}. {path}:{lineno}\n\n```\n" + "\n".join(ctx) + "\n```\n")

OUT.write_text(
    f"# `narrative` sample — {len(picked)} findings "
    f"({len(md)} md and {len(rs)} rs in the population)\n\n"
    "The flagged line is marked `>>`. Classify each.\n\n" + "\n".join(blocks),
    encoding="utf-8",
)
print(f"wrote {OUT} — {len(picked)} findings, population {len(findings)}")
