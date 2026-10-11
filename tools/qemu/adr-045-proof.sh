#!/bin/sh
# tools/qemu/adr-045-proof.sh — **the supervisor really ASKS, and a wedged
# application is really reported as not answering.**
#
# `ADR-045` §11.1 steps 1 and 1a. Run it in the container:
#
#   RIVET_CACHE_TAG=-health tools/container/exec.sh tools/qemu/adr-045-proof.sh
#
# ## What it asserts, and why each half is needed
#
# **Step 1** — the supervisor polls an application, it answers `degraded` with a
# sentence, and `rivet get status` says so. Over the real control plane, with
# `cargo rivet run`, because a harness speaking its own dialect could pass
# against a machine no operator could reach.
#
# **Step 1a** — the SAME program, installed under a name with `wedge` in it,
# answers `healthy` once and then `SIGSTOP`s itself. Its pid does not change, it
# does not exit, its restart count stays zero. **This is the test the telling
# design could not pass**, and it is the reason `ADR-045` §3.1 was reversed on
# 2026-09-08: under a design where the application speaks first, a process in
# this state goes on being reported as the last thing it said.
#
# **And the fold** — `ADR-045` §4.4. The document marks the wedging probe
# `critical` and the degrading one not, so the MACHINE's answer must be
# `healthy` while the wedge is answering and `unknown` after it stops, and the
# `degraded` probe must not move it at any point. That is §4.4's first stated
# consequence, live.
#
# ## One boot, two applications, and that is the control
#
# `tools/qemu/adr-051-proof.sh` boots twice because the thing it varies is a
# manifest. Here the thing that varies is the APPLICATION, so both run at once
# under one supervisor: a run in which one answers and the other does not is a
# run in which the difference cannot be the boot, the kernel, the image or the
# clock.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column, entirely: no Raspberry Pi, no SD card, no
# real timing. And it says nothing about the fleet server (`ADR-045` §6, §11.1
# step 3) or the commit gate (§7, step 2) — those are the hops above this one
# and are unbuilt.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO/target` — `tools/container/exec.sh` mounts a
# named volume there and exports it, so a script that composed the in-tree path
# would build correctly and then look where nothing put the binaries.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-045-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-045-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-045-proof: $1" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Constants read out of the source, never written here
# ---------------------------------------------------------------------------
#
# `tools/qemu/run.sh`'s `crate_path` argument, and the same one: a harness that
# restated a path would pack a file where nothing looks for it the day a ruling
# moves it, and the symptom would be a failure path running quietly in a run
# meant to demonstrate the success path.
constant() {
    file=$1
    name=$2
    value=$(sed -n "s/^pub const $name: *&str *= *\"\([^\"]*\)\".*/\1/p" "$file" | head -1)
    [ -n "$value" ] || die "no $name in $file — refusing to guess it"
    printf '%s\n' "$value"
}
number() {
    file=$1
    name=$2
    value=$(sed -n "s/^pub const $name: *u\(16\|64\) *= *\([0-9_]*\).*/\2/p" "$file" \
        | tr -d '_' | head -1)
    [ -n "$value" ] || die "no $name in $file — refusing to guess it"
    printf '%s\n' "$value"
}

DEFINITIONS_RS=$REPO/cmd/rivet-sysd/src/definitions.rs
CONTROL_PLANE_RS=$REPO/rivet-config/src/control_plane.rs
HEALTH_RS=$REPO/rivet-core/src/health.rs
MCPD_PEERS_PATH=$(constant "$DEFINITIONS_RS" MCPD_PEERS_PATH)
CONTROL_PORT=$(number "$CONTROL_PLANE_RS" DEFAULT_PORT)
CHANNEL_NAME=$(constant "$HEALTH_RS" CHANNEL_NAME)
INTERVAL_MS=$(number "$HEALTH_RS" INTERVAL_MS)
PATIENCE_MS=$(number "$HEALTH_RS" PATIENCE_MS)
echo "adr-045-proof: the channel is \`$CHANNEL_NAME\`; the supervisor asks every ${INTERVAL_MS}ms"
echo "adr-045-proof: and gives up after ${PATIENCE_MS}ms"

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — rivetos-mcpd links ring, whose build compiles C \
(ADR-009 §3.7.4). Point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-045-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd ) \
    || die "the guest's binaries did not build"

echo "adr-045-proof: building the health probe"
( cd "$HERE/../../examples/probes/health-probe" \
    && cargo build --release --target "$TARGET" ) \
    || die "examples/probes/health-probe did not build"
PROBE=$OUT/health-probe
[ -f "$PROBE" ] || die "the probe was not built at $PROBE"

echo "adr-045-proof: building cargo-rivet for the HOST — the control-plane client"
( cd "$REPO" && cargo build --release -p cargo-rivet ) \
    || die "cargo-rivet did not build; without it this proof cannot ask the guest anything"
# **The repository's own, by path.** The `cargo-rivet` on PATH inside the
# container is stale and refuses manifest keys this build writes.
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet

( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer

# ---------------------------------------------------------------------------
# Two installed applications, from ONE binary
# ---------------------------------------------------------------------------
#
# The probe picks its behaviour from the name it is installed under —
# `examples/probes/health-probe/src/main.rs`. Copying one binary to two names is
# what makes this a single-variable experiment.
ANSWERS=health-probe
WEDGES=health-wedge-probe
cp "$PROBE" "$WORK/$ANSWERS"
cp "$PROBE" "$WORK/$WEDGES"

# **One manifest, used by both**, so nothing about the difference below can be
# attributed to a declaration. Written as the canonical text `Manifest::render`
# emits, in `KEYS` order, because that is what an installer leaves on disk.
#
# **Note what it does NOT declare: a socket.** `ADR-045` §3.1.2 — the manifest
# does not ask for the answering channel and every application gets one. A
# manifest with a `listen` in it would leave the reader unable to tell which
# descriptor the probe answered on.
printf '%s' 'kind = "daemon"
restart = "always"
ready = "notify"
network = "none"
state = "private"
listen = []
provides = []
connects = []
library = []
devices = []
' >"$WORK/manifest.toml"

"$SIGNER" "$ANSWERS" "$WORK/$ANSWERS" "$WORK/manifest.toml" \
    "$WORK/header-answers" "$WORK/trust" >/dev/null \
    || die "the answering probe could not be signed"
"$SIGNER" "$WEDGES" "$WORK/$WEDGES" "$WORK/manifest.toml" \
    "$WORK/header-wedges" "$WORK/trust" >/dev/null \
    || die "the wedging probe could not be signed"

# ---------------------------------------------------------------------------
# Two throwaway identities, minted per run
# ---------------------------------------------------------------------------
#
# `AGENTS.md` §8.3 keeps key material out of tracked files, and a committed
# `device.seed` would be a private key every reader of this repository holds.
# `cargo rivet identity new` is the only thing in the workspace that mints one.
echo "adr-045-proof: minting THROWAWAY identities in $WORK"
"$CARGO_RIVET" identity new --directory "$WORK/guest" --name adr045-guest \
    >"$WORK/mint-guest" 2>&1 || { cat "$WORK/mint-guest" >&2; die "no guest identity"; }
"$CARGO_RIVET" identity new --directory "$WORK/operator" --name adr045-operator \
    >"$WORK/mint-operator" 2>&1 || { cat "$WORK/mint-operator" >&2; die "no operator identity"; }
GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/guest" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
[ -n "$GUEST_PIN" ] || die "could not read the guest's public key back out of its certificate"
[ -n "$OPERATOR_PIN" ] || die "could not read the operator's public key"
[ "$GUEST_PIN" != "$OPERATOR_PIN" ] \
    || die "the two identities minted the same key, so nothing below distinguishes the ends"

mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/" \
    || die "could not assemble the guest's identity directory"
printf '# one hex public key per line\n%s\n' "$OPERATOR_PIN" \
    >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

# ---------------------------------------------------------------------------
# The document — and the `critical` key is the point of it
# ---------------------------------------------------------------------------
#
# **`ADR-045` §4.3: `critical` is the OPERATOR's word, in the machine's
# document.** Neither manifest above says anything about importance and neither
# could; this file is where the difference is stated, and it is stated about the
# WEDGING probe rather than the degrading one — so the machine's answer is moved
# by the service that stops answering and is NOT moved by the service that says
# it is unwell. That is §4.4's first stated consequence, as an experiment.
DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "adr045-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[services.%s]\nenabled = true\n' "$ANSWERS"
    printf '\n[services.%s]\nenabled = true\ncritical = true\n' "$WEDGES"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"
[ "$(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC")" -eq 1 ] \
    || { cat -n "$DOC" >&2; die "the composed document has more than one \`version\` key"; }
grep -q '^critical = true$' "$DOC" \
    || die "the \`critical\` key did not reach $DOC, so the fold assertion would pass vacuously"

# ---------------------------------------------------------------------------
# Boot
# ---------------------------------------------------------------------------
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

LOG=$WORK/console.log
echo "adr-045-proof: booting with both probes installed, control plane on $CONTROL_PORT"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --secrets "$WORK/secrets" \
    --document "$DOC" \
    --app "$ANSWERS:$WORK/$ANSWERS:$WORK/manifest.toml:$WORK/header-answers" \
    --app "$WEDGES:$WORK/$WEDGES:$WORK/manifest.toml:$WORK/header-wedges" \
    --trust "$WORK/trust" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
    >"$LOG" 2>&1 </dev/null &
QEMU=$!
# shellcheck disable=SC2064
trap "kill $QEMU 2>/dev/null || true" EXIT

# Wait for the DAEMON's own line rather than for a number of seconds: it is the
# earliest moment a connection could be accepted.
WAITED=0
while [ "$WAITED" -lt 240 ]; do
    grep -qF 'rivet-mcpd: listening on' "$LOG" 2>/dev/null && break
    if ! kill -0 "$QEMU" 2>/dev/null; then
        cat "$LOG" >&2
        die "run.sh exited before the control plane reported listening — see above"
    fi
    sleep 0.5
    WAITED=$((WAITED + 1))
done
[ "$WAITED" -lt 240 ] || { tail -60 "$LOG" >&2; die "the control plane never reported listening"; }
echo "adr-045-proof: the control plane is up after ${WAITED}00ms"

ask() {
    set +e
    "$CARGO_RIVET" run \
        --host "127.0.0.1:$CONTROL_PORT" \
        --identity "$WORK/operator" \
        --pin "$GUEST_PIN" \
        -- get status \
        >"$WORK/status" 2>"$WORK/status-err"
    RC=$?
    set -e
}

# The first ask is retried, and **only for a failure to reach the machine at
# all**: `rivet-mcpd` binds before `rivet-netd` has necessarily given eth0 its
# address. A refusal from the machine is an ANSWER and is reported first time.
ATTEMPT=1
while :; do
    ask
    [ "$RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 6 ] && break
    grep -qiE 'connection (refused|reset)|timed out|os error' "$WORK/status-err" || break
    echo "adr-045-proof: the guest is not reachable yet (attempt $ATTEMPT)"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$RC" -ne 0 ]; then
    cat "$WORK/status-err" >&2
    grep -aF 'rivet-mcpd:' "$LOG" >&2 || true
    die "\`cargo rivet run -- get status\` exited $RC"
fi

# --- reading one service out of the answer ---------------------------------
#
# `Status::value`'s JSON, one object per service, `name` first. Cut the object
# and read its `health` — never the machine's, which carries the same key one
# level up. A grep for `"health"` alone would find whichever came first and
# would have been right by accident on most runs.
health_of() {
    tr -d '\n' <"$WORK/status" \
        | grep -o "\"name\":\"$1\"[^}]*" \
        | grep -o '"health":"[a-z]*"' \
        | head -1 \
        | sed 's/.*:"\([a-z]*\)"/\1/'
}
caption_of() {
    tr -d '\n' <"$WORK/status" \
        | grep -o "\"name\":\"$1\"[^}]*" \
        | grep -o '"caption":"[^"]*"' \
        | head -1 \
        | sed 's/.*:"\([^"]*\)"/\1/'
}
machine_health() {
    # The machine's own word is the FIRST `health` in the document: `value()`
    # puts it beside `phase`, before the services list.
    tr -d '\n' <"$WORK/status" \
        | grep -o '"health":"[a-z]*"' \
        | head -1 \
        | sed 's/.*:"\([a-z]*\)"/\1/'
}
expect() {
    what=$1
    got=$2
    want=$3
    if [ "$got" = "$want" ]; then
        echo "  OK   $what is \`$want\`"
    else
        fail "$what is \`${got:-(absent)}\` and must be \`$want\`"
    fi
}

# ---------------------------------------------------------------------------
# Assertion 1 — the supervisor asked, both answered, and one says `degraded`
# ---------------------------------------------------------------------------
#
# The degrading probe concludes at five seconds and sends `ADR-045` §3.1.3's
# *ask me now*, so the supervisor polls it without waiting for its interval.
echo
echo "--- ADR-045 §11.1 step 1: it answers, and \`get status\` says so ---"
sleep 12
ask
[ "$RC" -eq 0 ] || { cat "$WORK/status-err" >&2; die "the second ask failed"; }
cat "$WORK/status" >"$WORK/status-1"

expect "$ANSWERS" "$(health_of "$ANSWERS")" degraded
CAPTION=$(caption_of "$ANSWERS")
case $CAPTION in
    "no reading in "*s) echo "  OK   its caption is \"$CAPTION\" — a state WITH A SUBJECT (ADR-045 §5)" ;;
    *) fail "the caption is \"${CAPTION:-(absent)}\" and must name what is wrong" ;;
esac

# **The control.** The wedging probe is still answering at this point, which is
# what makes the next assertion about a service that STOPPED answering rather
# than one that never answered.
expect "$WEDGES (before it wedges)" "$(health_of "$WEDGES")" healthy

# **The fold, first half** — `ADR-045` §4.4. `health-probe` is `degraded` and is
# not `critical`, so the MACHINE is healthy. A build that folded *the worst of
# them* fails here.
expect "the MACHINE, with a degraded NON-critical service" "$(machine_health)" healthy

# ---------------------------------------------------------------------------
# Assertion 1a — WEDGED, and reported as not answering rather than as healthy
# ---------------------------------------------------------------------------
#
# The probe SIGSTOPped itself at six seconds. The supervisor's next question is
# due one INTERVAL after its first, and it gives up PATIENCE later.
echo
echo "--- ADR-045 §11.1 step 1a: WEDGED, and it is not reported healthy ---"
grep -aq 'WEDGING NOW' "$LOG" \
    || fail "the wedging probe never said it was stopping itself; nothing below is about a wedge"
WAIT_S=$(( (INTERVAL_MS + PATIENCE_MS) / 1000 + 6 ))
echo "adr-045-proof: waiting ${WAIT_S}s — one interval plus the supervisor's patience"
sleep "$WAIT_S"
ask
[ "$RC" -eq 0 ] || { cat "$WORK/status-err" >&2; die "the third ask failed"; }
cat "$WORK/status" >"$WORK/status-2"

expect "$WEDGES (wedged)" "$(health_of "$WEDGES")" unknown

# **And every signal the telling design had still says it is fine.** This is
# what makes the assertion above worth anything: the process did not exit, the
# supervisor still holds its pid, and its restart count is zero.
WEDGE_ROW=$(tr -d '\n' <"$WORK/status" | grep -o "\"name\":\"$WEDGES\"[^}]*")
case $WEDGE_ROW in
    *'"state":"started"'*) echo "  OK   its state is still \`started\`" ;;
    *) fail "the wedged probe is not \`started\` — it exited, so this proves nothing about a WEDGE: $WEDGE_ROW" ;;
esac
case $WEDGE_ROW in
    *'"restarts":0'*) echo "  OK   its restart count is still 0" ;;
    *) fail "the wedged probe has been restarted, so it is not the case under test: $WEDGE_ROW" ;;
esac
case $WEDGE_ROW in
    *'"pid":'*) echo "  OK   the supervisor still holds its pid" ;;
    *) fail "the wedged probe has no pid: $WEDGE_ROW" ;;
esac

# **The fold, second half.** The wedge IS `critical`, so the machine follows it
# — and `unknown` outranks the `degraded` the other probe is still reporting,
# which is the ordering clause `ADR-045` §4.4 argues for.
expect "the MACHINE, with a critical service that stopped answering" "$(machine_health)" unknown
expect "$ANSWERS (still)" "$(health_of "$ANSWERS")" degraded

# ---------------------------------------------------------------------------
# The controls
# ---------------------------------------------------------------------------
echo
echo "--- the controls ---"
if grep -aq "$CHANNEL_NAME" "$LOG"; then
    echo "  OK   the reserved channel name \`$CHANNEL_NAME\` appears on the console"
else
    fail "\`$CHANNEL_NAME\` appears nowhere in the console log, so nothing shows the descriptor was handed over"
fi
if grep -aq 'did not answer the health question' "$LOG"; then
    echo "  OK   the supervisor said, in its own journal, that a service stopped answering"
else
    fail "the supervisor never journalled the silence, so the status word above has no account behind it"
fi
if diff -q "$WORK/status-1" "$WORK/status-2" >/dev/null 2>&1; then
    fail "the two answers are byte-identical, so nothing changed between them and both assertions read one moment"
else
    echo "  OK   the two answers differ, so the machine changed between them"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-045-proof: PASS — it answered, and wedged it was reported as not answering"
else
    echo "adr-045-proof: FAIL — the console log and both answers are in $WORK" >&2
fi
exit "$VERDICT"
