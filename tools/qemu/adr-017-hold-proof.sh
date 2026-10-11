#!/bin/sh
# tools/qemu/adr-017-hold-proof.sh — **one application is stopped, stays
# stopped, and is started again, over the real control plane.**
#
# `ADR-017` §6.5. Run it in the container:
#
#   tools/container/exec.sh tools/qemu/adr-017-hold-proof.sh
#
# ## What it asserts
#
# * `apply service-stop rivet-mcpd confirm` is REFUSED, `E_UNSAFE`, and the
#   control plane is still answering afterwards;
# * the unconfirmed `apply service-stop <app>` is a plan that names
#   `/state/app/<app>` as untouched;
# * the confirmed stop is taken, and after longer than the restart backoff the
#   application is `idle`, has no pid, still reads `enabled: true`, and gives the
#   hold as its reason — the supervisor did NOT restart it as a crash;
# * `/state/app/<app>` still holds `current` and `data`;
# * `apply service-restart <app> confirm` is refused while it is held;
# * `apply service-start <app> confirm` is taken and the application runs again
#   under a NEW pid.
#
# Every assertion is read from the machine's own answer, through `cargo rivet
# run` — the client an operator holds.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column: no Raspberry Pi, no SD card. And it does not
# reboot the guest, so that a reboot releases a hold rests on the unit test that
# a fresh supervisor holds nothing. The two identities and the trust store are
# THROWAWAY, minted into the container's temporary directory on every run, as
# `adr-045-proof.sh` does — no key of anybody's is read.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-017-hold-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-017-hold-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-017-hold-proof: $1" >&2
    exit 1
}
ok() {
    echo "  OK   $1"
}

# Constants read out of the source, never written here — `adr-045-proof.sh`'s
# reason.
constant() {
    value=$(sed -n "s/^pub const $2: *&str *= *\"\([^\"]*\)\".*/\1/p" "$1" | head -1)
    [ -n "$value" ] || die "no $2 in $1 — refusing to guess it"
    printf '%s\n' "$value"
}
number() {
    value=$(sed -n "s/^pub const $2: *u\(16\|64\) *= *\([0-9_]*\).*/\2/p" "$1" \
        | tr -d '_' | head -1)
    [ -n "$value" ] || die "no $2 in $1 — refusing to guess it"
    printf '%s\n' "$value"
}
MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
CONTROL_PORT=$(number "$REPO/rivet-config/src/control_plane.rs" DEFAULT_PORT)
HOLD_RS=$REPO/rivet-core/src/operation/service_hold.rs
STOP=$(constant "$HOLD_RS" STOP_SUBJECT)
START=$(constant "$HOLD_RS" START_SUBJECT)

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-017-hold-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd ) \
    || die "the guest's binaries did not build"
( cd "$REPO/examples/probes/health-probe" && cargo build --release --target "$TARGET" ) \
    || die "examples/probes/health-probe did not build"
PROBE=$OUT/health-probe
[ -f "$PROBE" ] || die "the probe was not built at $PROBE"
( cd "$REPO" && cargo build --release -p cargo-rivet ) || die "cargo-rivet did not build"
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet
( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer

# ---------------------------------------------------------------------------
# One installed application, signed by a throwaway key
# ---------------------------------------------------------------------------
APP=health-probe
cp "$PROBE" "$WORK/$APP"
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
"$SIGNER" "$APP" "$WORK/$APP" "$WORK/manifest.toml" "$WORK/header" "$WORK/trust" >/dev/null \
    || die "the probe could not be signed"

echo "adr-017-hold-proof: minting THROWAWAY identities in $WORK"
"$CARGO_RIVET" identity new --directory "$WORK/guest" --name hold-guest \
    >"$WORK/mint-guest" 2>&1 || { cat "$WORK/mint-guest" >&2; die "no guest identity"; }
"$CARGO_RIVET" identity new --directory "$WORK/operator" --name hold-operator \
    >"$WORK/mint-operator" 2>&1 || { cat "$WORK/mint-operator" >&2; die "no operator identity"; }
GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/guest" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
[ -n "$GUEST_PIN" ] || die "could not read the guest's public key"
[ -n "$OPERATOR_PIN" ] || die "could not read the operator's public key"
mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/" \
    || die "could not assemble the guest's identity directory"
printf '# one hex public key per line\n%s\n' "$OPERATOR_PIN" \
    >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "hold-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[services.%s]\nenabled = true\n' "$APP"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"

# ---------------------------------------------------------------------------
# Boot
# ---------------------------------------------------------------------------
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

LOG=$WORK/console.log
echo "adr-017-hold-proof: booting with $APP installed, control plane on $CONTROL_PORT"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --secrets "$WORK/secrets" \
    --document "$DOC" \
    --app "$APP:$WORK/$APP:$WORK/manifest.toml:$WORK/header" \
    --trust "$WORK/trust" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
    >"$LOG" 2>&1 </dev/null &
QEMU=$!
# shellcheck disable=SC2064
trap "kill $QEMU 2>/dev/null || true" EXIT

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

# `rivet` $@ over the control plane. Sets RC; stdout and stderr land in files.
rivet() {
    set +e
    "$CARGO_RIVET" run --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/operator" \
        --pin "$GUEST_PIN" -- "$@" >"$WORK/out" 2>"$WORK/err"
    RC=$?
    set -e
}

ATTEMPT=1
while :; do
    rivet get status
    [ "$RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 6 ] && break
    grep -qiE 'connection (refused|reset)|timed out|os error' "$WORK/err" || break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
[ "$RC" -eq 0 ] || { cat "$WORK/err" >&2; die "\`get status\` exited $RC"; }

row() {
    tr -d '\n' <"$WORK/out" | grep -o "\"name\":\"$APP\"[^}]*" | head -1
}
pid_of() {
    row | grep -o '"pid":[0-9]*' | head -1 | sed 's/.*://'
}

# Let the probe reach `started`.
sleep 6
rivet get status
FIRST_PID=$(pid_of)
case $(row) in
    *'"state":"started"'*) ok "$APP is started, pid $FIRST_PID" ;;
    *) tail -40 "$LOG" >&2; die "$APP is not started before the stop, so nothing below means anything: $(row)" ;;
esac

# ---------------------------------------------------------------------------
# The control plane cannot stop itself
# ---------------------------------------------------------------------------
echo
echo "--- $STOP rivet-mcpd is refused ---"
rivet apply "$STOP" rivet-mcpd confirm
if [ "$RC" -ne 0 ] && grep -q 'E_UNSAFE' "$WORK/err" && grep -q 'REACHABLE' "$WORK/err"; then
    ok "refused, E_UNSAFE, naming reachability"
else
    fail "\`$STOP rivet-mcpd confirm\` exited $RC: $(cat "$WORK/out" "$WORK/err")"
fi
rivet get status
[ "$RC" -eq 0 ] && ok "the control plane still answers" || fail "the control plane stopped answering"

# ---------------------------------------------------------------------------
# The plan, then the stop
# ---------------------------------------------------------------------------
echo
echo "--- $STOP $APP ---"
rivet apply "$STOP" "$APP"
if [ "$RC" -ne 0 ] && grep -q 'E_CONFIRMATION_REQUIRED' "$WORK/err" \
    && grep -q "/state/app/$APP" "$WORK/err"; then
    ok "unconfirmed, it is a plan naming /state/app/$APP as untouched"
else
    fail "the unconfirmed stop was not a plan: $(cat "$WORK/out" "$WORK/err")"
fi

rivet apply "$STOP" "$APP" confirm
if [ "$RC" -eq 0 ] && grep -q '"doing":"stop"' "$WORK/out"; then
    ok "the stop is taken: $(tr -d '\n' <"$WORK/out" | grep -o '"doing":"[a-z]*"')"
else
    fail "the confirmed stop exited $RC: $(cat "$WORK/out" "$WORK/err")"
fi

# Longer than the first backoff several times over — a restart-as-crash would
# have started it again inside this.
sleep 20
rivet get status
ROW=$(row)
case $ROW in
    *'"state":"idle"'*) ok "after 20s it is idle — not restarted" ;;
    *) fail "after 20s it is not idle: $ROW" ;;
esac
case $ROW in
    *'"pid":'*) fail "it has a pid, so something is running: $ROW" ;;
    *) ok "it has no pid" ;;
esac
case $ROW in
    *'"enabled":true'*) ok "enabled is still true — the document is untouched" ;;
    *) fail "enabled is not true: $ROW" ;;
esac
case $ROW in
    *"$START"*) ok "its reason names \`$START\`" ;;
    *) fail "its reason does not give the hold: $ROW" ;;
esac

rivet get file "/state/app/$APP"
if [ "$RC" -eq 0 ] && grep -q 'current' "$WORK/out" && grep -q 'data' "$WORK/out"; then
    ok "/state/app/$APP still holds current and data"
else
    fail "/state/app/$APP was changed: $(cat "$WORK/out" "$WORK/err")"
fi

rivet apply service-restart "$APP" confirm
if [ "$RC" -ne 0 ] && grep -q 'E_UNSAFE' "$WORK/err"; then
    ok "service-restart is refused while it is held"
else
    fail "service-restart was not refused while held: $(cat "$WORK/out" "$WORK/err")"
fi

# ---------------------------------------------------------------------------
# The start
# ---------------------------------------------------------------------------
echo
echo "--- $START $APP ---"
rivet apply "$START" "$APP" confirm
if [ "$RC" -eq 0 ] && grep -q '"doing":"start"' "$WORK/out"; then
    ok "the start is taken"
else
    fail "the confirmed start exited $RC: $(cat "$WORK/out" "$WORK/err")"
fi
sleep 6
rivet get status
SECOND_PID=$(pid_of)
case $(row) in
    *'"state":"started"'*) ok "it is started again, pid $SECOND_PID" ;;
    *) fail "it is not started after $START: $(row)" ;;
esac
if [ -n "$SECOND_PID" ] && [ "$SECOND_PID" != "$FIRST_PID" ]; then
    ok "under a new pid"
else
    fail "the pid did not change ($FIRST_PID then ${SECOND_PID:-none})"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-017-hold-proof: PASS — stopped, held, and started again"
else
    echo "adr-017-hold-proof: FAIL — the console log is at $LOG" >&2
    cp "$LOG" "$REPO/out/adr-017-hold-proof.console.log" 2>/dev/null || true
fi
exit "$VERDICT"
