#!/bin/sh
# tools/qemu/adr-007-cpu-proof.sh — **a CPU cap is a cgroup, read back, not a
# byte string nobody checked.**
#
# `ADR-007` §7.2.5 and `ADR-015` §3.2.3, ruled 2026-09-09: `cpu` joins
# `memory` as an operator key and a manifest request, on the identical
# split-by-direction footing. Run it in the container:
#
#   RIVET_CACHE_TAG=-cpu tools/container/exec.sh tools/qemu/adr-007-cpu-proof.sh
#
# ## Why a console-only boot, on `adr-042-proof.sh`'s shape
#
# The question here is answered entirely by what a running kernel says about
# its own cgroups — no control plane needed, no `rivet` CLI, no identities.
# `adr-045-proof.sh` and `adr-042-net-proof.sh` reach for the control plane
# because their question is what `rivet get status` reports; this one reads
# `cpu.stat` directly and prints it to the console, so the lighter boot
# `adr-042-proof.sh` uses is the right one to copy.
#
# ## Why a SYSTEM service slot, and not a signed application — the choice
# that shapes the whole script
#
# An installed application is Landlock-confined (`ADR-042`), and
# `rivet_core::confinement`'s own documentation is explicit that `/proc` and
# `/sys` are granted **nowhere beyond one file** — so a probe packaged as an
# application could never open its own `/sys/fs/cgroup/rivet/<name>/cpu.stat`
# to prove anything; the read would fail with `EACCES` regardless of whether
# the cap enforced, and a script that could not tell "refused by Landlock"
# from "refused by the cap not applying" would prove nothing.
#
# A system service that runs as uid 0 with no Landlock policy at all
# (`crate::service::IMAGE_DEFINED`'s `confinement: None`) can read its own
# cgroup, and `[services.<name>]`'s `cpu` key caps a system service exactly as
# it caps an application — the document does not know or care which kind of
# service a name belongs to; `cmd/rivet-sysd/src/document.rs`'s `apply_cpu_cap`
# and `crate::supervise`'s own of the same name run unchanged either way. So
# `tools/qemu/cpu-spin-probe` is installed in place of `rivet-netd` (capped
# at `cpu = "0.5"`, already `enabled` by image default) for the FIRST half and
# `rivet-mcpd` (enabled by image default, no `cpu` key at all) for the SECOND —
# `tools/qemu/run.sh`'s own documented use of `--netd` and `--mcpd`:
# substituting the binary at a path the supervisor already knows how to run.
# `rivet-mcpd` restarts `always`, and the probe parks rather than exits, so it
# is never restarted mid-measurement.
#
# **Both slots must be UNCONFINED image services**, and that is what decides
# which two. A homed image service (`ADR-017` §4.1) runs under its own uid and
# the image-service ruleset, which grants nothing under `/sys`, so the probe in
# its slot reads `Permission denied` for its own `cpu.max` and `cpu.stat`. The
# capped half used `rivet-timed`'s slot until timed became homed, and the
# uncapped half `rivet-bluetoothd`'s until it did. `rivet-netd` runs once and
# exits, so `on-failure` never restarts the parked probe either.
# If either slot becomes homed, this proof moves to another unconfined one.
#
# ## The two halves, and why one alone is worthless
#
# 1. **A capped service is actually capped.** The probe spins one core with a
#    tight arithmetic loop for six wall-clock seconds and reads its own
#    cgroup's `cpu.stat` before and after — `usage_usec`'s delta over that
#    exact window is the kernel's own account of how much CPU it was given,
#    not an assertion from documentation (`AGENTS.md` §5).
# 2. **Nothing that ought to start stops starting, and a service with no `cpu`
#    declared is unaffected.** `rivet-mcpd` gets no `cpu` key; this asserts
#    it still starts, still spins to near the full
#    core, and the supervisor's own journal never writes a cpu cap for it —
#    the failure mode that matters most, because getting this wrong is a
#    board where a service will not run.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column, entirely: no Raspberry Pi, no SD card, and
# QEMU's `-M virt` vCPU is not a Raspberry Pi 4's Cortex-A72 — the cap's
# *arithmetic* is proven, nothing about how close a real board's scheduler
# tracks the same quota under real contention.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# **`CARGO_TARGET_DIR` and not `$REPO/target`** — `tools/container/exec.sh`
# mounts a named volume there and exports it, so a script that composed the
# in-tree path would build correctly and then look where nothing put the
# binaries. The probe crate is its own workspace root (see its `Cargo.toml`)
# and lands in the SAME volume because `CARGO_TARGET_DIR` is process-wide,
# not workspace-scoped — `adr-042-proof.sh`'s own comment on this.
OUT=${CARGO_TARGET_DIR:-$REPO/target}/$TARGET/release

VERDICT=0
fail() {
    echo "adr-007-cpu-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-007-cpu-proof: $1" >&2
    exit 1
}

# `rivetos-sysd` links `ring` through `rivetos-tls`, whose build compiles C, so
# `cc-rs` is told which compiler — the same two lines `adr-017-hold-proof.sh`
# and `smoke.sh` carry, for the same reason.
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-007-cpu-proof: building pid 1, the supervisor and the probe for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" -p rivetos-initd -p rivetos-sysd ) \
    || die "the guest's binaries did not build"

# The probe is its own workspace (`tools/qemu/cpu-spin-probe/Cargo.toml` says
# why, on `testapp`'s footing), so it is built on its own.
( cd "$HERE/cpu-spin-probe" && cargo build --release --target "$TARGET" ) \
    || die "the probe did not build"
PROBE=$OUT/rivet-cpu-spin-probe
[ -f "$PROBE" ] || die "the probe was not built at $PROBE"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

SECONDS_TO_RUN=${RIVET_PROOF_SECONDS:-90}
LOG=${RIVET_PROOF_LOG:-${TMPDIR:-/tmp}/adr-007-cpu-proof.log}

echo "adr-007-cpu-proof: booting, bounded at ${SECONDS_TO_RUN}s, console to $LOG"
echo "adr-007-cpu-proof: rivet-netd (capped cpu=0.5) and rivet-mcpd (uncapped) both run the probe"
set +e
timeout -k 2 "$SECONDS_TO_RUN" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --netd "$PROBE" \
    --mcpd "$PROBE" \
    --document "$HERE/adr-007-cpu-proof.document.toml" \
    "$@" -smp 2 >"$LOG" 2>&1 </dev/null
RC=$?
set -e

echo
echo "--- the probes' own lines ---"
# **Not anchored at the start of a line** — `adr-042-proof.sh`'s reason: the
# supervisor re-emits a captured line as a JSON `msg` field, so the probe's
# words arrive inside quotes rather than at column 1.
grep -ao 'cpu-spin-probe: [^"]*' "$LOG" || {
    echo "adr-007-cpu-proof: NEITHER PROBE SAID ANYTHING. The whole console follows." >&2
    cat "$LOG" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Read the two RESULT lines back out of the console
# ---------------------------------------------------------------------------
#
# `RESULT name=<name> elapsed_us=<N> delta_usage_usec=<N>` — the probe's own
# format, chosen so this script parses its OWN report rather than re-deriving
# one from raw `cpu.stat` text scattered across several printed lines.
result_field() {
    name=$1
    field=$2
    grep -ao "RESULT name=$name elapsed_us=[0-9]*[^\"]*" "$LOG" \
        | tail -1 \
        | grep -o "$field=[0-9]*" \
        | tail -1 \
        | cut -d= -f2
}

TIMED_ELAPSED=$(result_field rivet-netd elapsed_us)
TIMED_DELTA=$(result_field rivet-netd delta_usage_usec)
BT_ELAPSED=$(result_field rivet-mcpd elapsed_us)
BT_DELTA=$(result_field rivet-mcpd delta_usage_usec)

echo
echo "--- what each probe measured, reading its own cgroup back ---"
echo "  rivet-netd       (cpu = \"0.5\"): elapsed_us=${TIMED_ELAPSED:-?} delta_usage_usec=${TIMED_DELTA:-?}"
echo "  rivet-mcpd        (no cpu key):  elapsed_us=${BT_ELAPSED:-?} delta_usage_usec=${BT_DELTA:-?}"

if [ -z "${TIMED_ELAPSED:-}" ] || [ -z "${TIMED_DELTA:-}" ]; then
    fail "rivet-netd never printed a RESULT line — either it did not start or could not read its own cpu.stat"
fi
if [ -z "${BT_ELAPSED:-}" ] || [ -z "${BT_DELTA:-}" ]; then
    fail "rivet-mcpd never printed a RESULT line — either it did not start or could not read its own cpu.stat"
fi

# ---------------------------------------------------------------------------
# Assertion 1 — the capped probe is actually capped
# ---------------------------------------------------------------------------
#
# `CpuLimit::PERIOD_US` is 100000; `cpu = "0.5"` is a quota of 50000, so the
# budget over the measured window is `elapsed_us * 0.5`. CFS bandwidth
# accounting is not exact instantaneous throttling, so this allows 40% slack
# above the theoretical budget before calling it uncapped, and separately
# requires it to be MEANINGFULLY below what the uncapped run below achieves —
# the two thresholds together are what rule out both "the cap did nothing"
# and "the whole guest happened to be slow this boot".
echo
echo "--- ADR-007 §7.2.5 / ADR-015 §3.2.3: a capped service is actually capped ---"
if [ -n "${TIMED_ELAPSED:-}" ] && [ -n "${TIMED_DELTA:-}" ]; then
    BUDGET=$((TIMED_ELAPSED / 2))
    CEILING=$((BUDGET + BUDGET * 4 / 10))
    if [ "$TIMED_DELTA" -le "$CEILING" ]; then
        echo "  OK   rivet-netd used ${TIMED_DELTA}us of CPU against a budget of ~${BUDGET}us (ceiling ${CEILING}us)"
    else
        fail "rivet-netd used ${TIMED_DELTA}us of CPU, which is more than ${CEILING}us — the 0.5-core cap did not hold"
    fi
fi

# ---------------------------------------------------------------------------
# Assertion 2 — nothing that ought to start stops starting, and the
# uncapped service is genuinely unaffected
# ---------------------------------------------------------------------------
echo
echo "--- ADR-007 §7.2.5 / ADR-015 §3.2.3: an unstated cpu is no cap at all ---"
if [ -n "${BT_ELAPSED:-}" ] && [ -n "${BT_DELTA:-}" ]; then
    FLOOR=$((BT_ELAPSED * 7 / 10))
    if [ "$BT_DELTA" -ge "$FLOOR" ]; then
        echo "  OK   rivet-mcpd used ${BT_DELTA}us of CPU over ${BT_ELAPSED}us elapsed — not throttled"
    else
        fail "rivet-mcpd used only ${BT_DELTA}us of ${BT_ELAPSED}us elapsed, which looks capped, and it declared no cpu key"
    fi
fi
if [ -n "${TIMED_DELTA:-}" ] && [ -n "${BT_DELTA:-}" ]; then
    if [ "$TIMED_DELTA" -lt "$BT_DELTA" ]; then
        echo "  OK   the capped probe used less CPU than the uncapped one over comparable windows"
    else
        fail "the capped probe (${TIMED_DELTA}us) did not use less CPU than the uncapped one (${BT_DELTA}us)"
    fi
fi

# ---------------------------------------------------------------------------
# The supervisor's own account — the cap was WRITTEN, and only where asked
# ---------------------------------------------------------------------------
echo
echo "--- the supervisor's own journal: what it wrote, and to which service ---"
if grep -aq '"svc":"rivet-netd"' "$LOG" && grep -aq '"msg":"capped"' "$LOG" \
    && grep -a '"svc":"rivet-netd"' "$LOG" | grep -aq '"cpu":"0.5"'; then
    echo "  OK   the supervisor journalled capping rivet-netd's cpu at 0.5"
else
    fail "no journal line shows the supervisor capping rivet-netd's cpu at 0.5 — the console follows on stderr" \
        && cat "$LOG" >&2
fi
if grep -a '"svc":"rivet-mcpd"' "$LOG" | grep -aq '"msg":"capped"'; then
    fail "the supervisor journalled a cpu cap for rivet-mcpd, which declared no cpu key at all"
else
    echo "  OK   the supervisor never journalled a cpu cap for rivet-mcpd"
fi

# **Both services still started, on the memory proof's own control** —
# `ADR-007` §7.2.4's worked example asserted the same shape about `memory`.
for svc in rivet-netd rivet-mcpd; do
    if grep -aq "\"svc\":\"$svc\".*\"msg\":\"started\"" "$LOG"; then
        echo "  OK   $svc started"
    else
        fail "$svc never reported \"started\" — nothing that ought to start may stop starting"
    fi
done

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-007-cpu-proof: PASS — a 0.5-core cap held, and an uncapped service ran unaffected"
else
    echo "adr-007-cpu-proof: FAIL — see above; the full console is in $LOG" >&2
fi
exit "$VERDICT"
