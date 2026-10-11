#!/bin/sh
# tools/qemu/adr-042-proof.sh — **the Landlock ruleset, proven both ways.**
#
# `ADR-042` §6. Run it in the container:
#
#   RIVET_CACHE_TAG=-lock2 tools/container/exec.sh tools/qemu/adr-042-proof.sh
#
# ## Why this exists as a script rather than a stage in `smoke.sh`
#
# `smoke.sh` is the gate and its two boots are about PID 1, `/etc`, the journal
# and the control plane. This asks one narrow question that needs its own
# document, its own installed application and a typeface in the initramfs, and
# folding all three into the gate would make every unrelated run pay for them.
# It is a script rather than a paragraph in a hand-off so that it can be RE-RUN
# — `AGENTS.md` §4: a test suite that cannot be run will rot silently.
#
# ## The two halves, and why one alone is worthless
#
# A confined application is started and reports, in one process, on one boot,
# at one uid:
#
# * every path the machine **handed** it — its settings file, its data
#   directory, its own installed manifest, and the image's typeface — which it
#   must be able to open; and
# * every path it **knows and never declared** — the operator's document,
#   the supervisor's own binary, `/state/app` and `/tmp` — which must be
#   refused with `EACCES`.
#
# **A probe that only showed the refusal could not tell a working ruleset from
# a missing file.** That is not hypothetical: it is how the regression of
# 2026-09-04 reached hardware. So every path in the second list EXISTS on the
# guest — `--document` plants the document, the initramfs carries the
# supervisor — and a refusal that comes back `ENOENT` is counted separately and
# reported as proving nothing.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column, entirely. There is no display here, so
# nothing about drawing is proven; the typeface is packed so that a READ can
# succeed, not so that anything renders. And the guest has **no `/dev/null` and
# no `/dev/kmsg`** (`AGENTS.md` §10.5) — the policy marks those
# `Presence::Optional` for exactly this reason, and their absence in the
# console is the harness, not a finding.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# **`CARGO_TARGET_DIR` and not `$REPO/target`.** `tools/container/exec.sh`
# mounts a named volume there and exports it, so a script that composed the
# in-tree path would build correctly and then look for the binaries somewhere
# nothing put them — and the fixture's own workspace lands in the same volume
# rather than beside its manifest.
OUT=${CARGO_TARGET_DIR:-$REPO/target}/$TARGET/release

echo "adr-042-proof: building the supervisor, pid 1 and the probe for $TARGET"
cargo build --release --target "$TARGET" -p rivetos-initd -p rivetos-sysd

# The fixture is its own workspace (`tools/qemu/testapp/Cargo.toml` says why),
# so it is built on its own and lands in its own target directory.
( cd "$HERE/testapp" && cargo build --release --target "$TARGET" )
PROBE=$OUT/rivet-testapp
if [ ! -f "$PROBE" ]; then
    echo "adr-042-proof: the probe was not built at $PROBE" >&2
    exit 1
fi

# **Installed under a name containing `confined`**, which is how this fixture
# chooses its behaviour, following `tools/qemu/testapp`'s precedent: the mode comes from `argv[0]`,
# because an application's definition comes from its signed manifest and there
# is no field an operator could put a mode in.
INSTALLED=${TMPDIR:-/tmp}/confinement-probe
cp "$PROBE" "$INSTALLED"

# **Signed, because the supervisor verifies before every launch.**
# `ADR-015` §4.2. Without a retained header and a trust store the supervisor
# defines the service and refuses to start it — correctly — and there is
# nothing to observe. `tools/qemu/fixture-signer` writes both with the real
# writer and the real signer; its key is a fixture and its own header says so.
HEADER=${TMPDIR:-/tmp}/confinement-probe.header
TRUST=${TMPDIR:-/tmp}/confinement-probe.trust
( cd "$HERE/fixture-signer" && cargo build --release )
"${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer" \
    confinement-probe \
    "$INSTALLED" \
    "$HERE/confinement-probe.manifest.toml" \
    "$HEADER" \
    "$TRUST"

# **Bounded, because PID 1 parks and nothing here ever exits** —
# `smoke.sh`'s own device, and its reasoning: `run.sh` execs
# `qemu-system-aarch64` with no intervening fork, so `timeout` signals QEMU
# itself. Exit 124 is the expected end of a good run.
# **The kernel is `ADR-024`'s own and is mounted, not built.**
# `tools/container/qemu.sh` exports `RIVET_KERNEL`; `exec.sh` mounts the same
# volume at `/br` and does not, so this fills it in rather than making the
# caller remember which wrapper sets what. Named explicitly and checked, so a
# volume with no `images/Image` in it refuses here with a sentence instead of
# inside `run.sh` with a path nobody chose.
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

SECONDS_TO_RUN=${RIVET_PROOF_SECONDS:-60}
LOG=${RIVET_PROOF_LOG:-/tmp/adr-042-proof.log}

echo "adr-042-proof: booting, bounded at ${SECONDS_TO_RUN}s, console to $LOG"
set +e
timeout -k 2 "$SECONDS_TO_RUN" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --document "$HERE/confinement-probe.document.toml" \
    --app "confinement-probe:$INSTALLED:$HERE/confinement-probe.manifest.toml:$HEADER" \
    --trust "$TRUST" \
    --font "$REPO/assets/fonts/Geist.ttf" \
    "$@" >"$LOG" 2>&1 </dev/null
RC=$?
set -e

echo "--- the probe's own lines ---"
# **Not anchored at the start of a line.** The supervisor captures an
# application's stdout (`ADR-022` §4) and re-emits each line as a journal
# record, so the probe's words arrive inside a JSON `msg` field rather than on
# their own. An anchored grep here found nothing and reported the probe silent
# on a boot where it had said everything.
grep -ao 'confined: [^"]*' "$LOG" || {
    echo "adr-042-proof: THE PROBE SAID NOTHING. The whole console follows." >&2
    cat "$LOG" >&2
    exit 1
}
echo "--- what the supervisor said it enforced ---"
grep -a 'landlock_abi' "$LOG" | head -4 || true

# **The control: those paths are reachable on this machine, by something else,
# on this boot.**
#
# Without it the second half is ambiguous — `AGENTS.md` §7. The probe already
# separates `EACCES` from `ENOENT`, which rules out *"the file was not there"*;
# this rules out *"the file was there and nothing could ever read it"*. The
# supervisor read `/state/config/machine.toml` on this same boot and says so by
# acting on it, and `/system/bin/rivet-sysd` is the program that is running.
#
# **It is a weaker control than the one in `cmd/rivet-sysd`'s `landlock`
# tests**, and the difference is worth naming rather than glossing: that one
# forks two children of one process at one uid and confines only the second, so
# the uid is held constant. This one holds the boot and the filesystem constant
# and not the uid. Both are in the record because neither is the other.
echo "--- the control: the same paths, read by something else, this boot ---"
if grep -aq 'the document overrides the definition' "$LOG"; then
    echo "control: the SUPERVISOR read /state/config/machine.toml on this boot"
    echo "         (it applied the document's enabled to the probe's definition),"
    echo "         so the EACCES above is a ruleset and not an unreadable file."
else
    echo "adr-042-proof: FAIL — nothing on this boot demonstrates the document was" >&2
    echo "    readable at all, so the probe's refusal proves nothing (AGENTS.md §7)" >&2
    exit 1
fi

# **The verdict is the probe's, not this script's.** It counted both halves and
# said which; re-deriving it here would be a second answer to one question.
if grep -aq 'confined: VERDICT .*BOTH HALVES' "$LOG"; then
    echo "adr-042-proof: PASS — both halves, exit $RC from the bounded run"
    exit 0
fi
echo "adr-042-proof: FAIL — the probe did not report both halves (exit $RC)" >&2
exit 1
