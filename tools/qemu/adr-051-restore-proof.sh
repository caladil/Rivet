#!/bin/sh
# tools/qemu/adr-051-restore-proof.sh — **an application's backup is fetched,
# its data changed, the backup restored, and a second backup is byte for byte
# the first — over the control plane on a booted machine, which reaches the
# application on loopback, with no client certificate presented by the tool.**
#
# `ADR-051` §8.1 and §12, `ADR-035` §3.3, `.rulings/` R-183 D1 and R-234. Run it
# in the container:
#
#   tools/container/exec.sh tools/qemu/adr-051-restore-proof.sh
#
# ## What is on the guest
#
# * `rivet-mcpd` and `rivet-netd` — the control plane `cargo rivet` reads the
#   backup table over.
# * `rivet-proxyd`, part of the image (`ADR-060`, run.sh's `--proxyd`), holding
#   443 (forwarded from the container's 8443), with `upstream-name` set in the
#   document.
# * `tools/qemu/restore-probe` installed TWICE: as `restore-probe`, whose
#   manifest declares `backup-path`, `backup-format = "sqlite"` and
#   `restore-path`; and as `plain-probe`, the same with no `restore-path`.
#
# The fleet authority, the operator's `browser` pair and the machine's server
# pair are minted by `rivet-provision ca` into this run's scratch directory and
# installed onto the guest with `cargo rivet secrets` — the production path. The
# control-plane identities are minted by `cargo rivet identity new`. Every key
# is THROWAWAY and none of anybody's is read.
#
# ## What it asserts
#
# * the supervisor's journal reports one application accepting a restore
#   (`"restores":1`) beside two declarations;
# * `cargo rivet backup` fetches `restore-probe`'s file over the control plane,
#   and `rivet-mcpd` journals the relay; one fetch runs with `HOME` pointed at
#   an empty directory, so no operator certificate is within the tool's reach;
# * `--authority`, `--name` and `--port` are refused by name;
# * the unconfirmed `cargo rivet restore` is a plan naming the route, and sends
#   nothing;
# * refused before anything is sent: `plain-probe` (no `restore-path`), a JSON
#   file (wrong format), and the backup cut short (right magic, wrong length);
# * after the application's data is changed, a second backup differs;
# * `rivet-proxyd` answers `403` to a restore `POST` carrying
#   `Sec-Fetch-Site: cross-site`, and to one carrying only an `Origin` from
#   another site; the data is unchanged after both, and the refusal is in the
#   journal; a `same-origin` `POST` carrying the machine's own `Origin` is
#   forwarded (`.rulings/` R-186 D1). The headers are set by `curl`, not by a
#   browser;
# * the confirmed restore reports the application's `200`, the application's
#   pid is the SAME afterwards (nothing was stopped), and a third backup is
#   byte-identical to the first.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column: no Raspberry Pi, no SD card, `/state` is the
# initramfs. And the application is a fixture built to SQLite's header
# arithmetic, not TempMon or Reliquary — each of those declares
# `restore-path`, and each one's consistency story (a live WAL database, a
# journal, a blob store) is proven only when that application runs it against
# its own real data. And the cross-site requests are `curl` carrying a
# browser's headers: **no browser is driven**.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-051-restore-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-051-restore-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-051-restore-proof: $1" >&2
    if [ -f "$WORK/console.log" ]; then
        mkdir -p "$REPO/out" 2>/dev/null || true
        cp "$WORK/console.log" "$REPO/out/adr-051-restore-proof.console.log" 2>/dev/null || true
    fi
    exit 1
}
ok() {
    echo "  OK   $1"
}

# Constants read out of the source, never written here.
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
PROXY_PORT=8443
GUEST=restore-guest
SUFFIX=local
MACHINE=$GUEST.$SUFFIX

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-051-restore-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd \
    -p rivetos-proxyd ) || die "the guest's binaries did not build"
( cd "$HERE/restore-probe" && cargo build --release --target "$TARGET" ) \
    || die "tools/qemu/restore-probe did not build"
PROBE=$OUT/restore-probe
[ -f "$PROBE" ] || die "the probe was not built at $PROBE"
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision ) \
    || die "the workstation tools did not build"
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet
PROVISION=$TARGET_DIR/release/rivet-provision
( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer

# ---------------------------------------------------------------------------
# Throwaway identities, and a throwaway fleet authority
# ---------------------------------------------------------------------------
echo "adr-051-restore-proof: minting THROWAWAY identities and an authority in $WORK"
mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$1" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; die "no $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
GUEST_PIN=$(mint guest)
OPERATOR_PIN=$(mint operator)
[ -n "$GUEST_PIN" ] && [ -n "$OPERATOR_PIN" ] || die "could not read a public key"
mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/"
printf '# the operator\n%s\n' "$OPERATOR_PIN" >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

# `HOME` points into the scratch directory for both calls, so nothing either
# does by default can reach a real ~/.rivet. The empty stdin answers the pfx
# password prompt with "none".
HOME="$WORK/home" "$PROVISION" ca "$SUFFIX" restore-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no authority"; }
HOME="$WORK/home" "$PROVISION" ca server "$GUEST" "$WORK/ca" \
    >>"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no server pair"; }
[ -f "$WORK/ca/server-$MACHINE.crt" ] || die "rivet-provision did not write server-$MACHINE.crt"

# ---------------------------------------------------------------------------
# Three installed applications, signed by the fixture key
# ---------------------------------------------------------------------------
# Canonical text in `KEYS` order, as `Manifest::render` writes it.
manifest_head() {
    printf 'kind = "daemon"\nrestart = "on-failure"\nready = "none"\nnetwork = "none"\n'
    printf 'state = "private"\nlisten = ["tcp:%s"]\nprovides = []\nconnects = []\n' "$1"
    printf 'library = []\ndevices = []\n'
}
{
    manifest_head 8080
    printf 'proxy-path = "probe"\nbackup-path = "api/v1/backup"\nbackup-format = "sqlite"\n'
    printf 'restore-path = "api/v1/restore"\n'
} >"$WORK/restore-probe.toml"
{
    manifest_head 8081
    printf 'proxy-path = "plain"\nbackup-path = "api/v1/backup"\nbackup-format = "sqlite"\n'
} >"$WORK/plain-probe.toml"

# **The proxy is part of the image** (`ADR-060`), packed at /system/bin by
# run.sh's `--proxyd`, and stripped as `tools/card/build.sh` strips it.
STRIP="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-strip"
[ -x "$STRIP" ] || die "no $STRIP to strip rivet-proxyd with"
"$STRIP" -o "$WORK/rivet-proxyd" "$OUT/rivet-proxyd" || die "rivet-proxyd could not be stripped"
cp "$PROBE" "$WORK/restore-probe"
cp "$PROBE" "$WORK/plain-probe"
for app in restore-probe plain-probe; do
    "$SIGNER" "$app" "$WORK/$app" "$WORK/$app.toml" "$WORK/$app.header" "$WORK/trust" >/dev/null \
        || die "$app could not be signed"
done

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for app in rivet-proxyd restore-probe plain-probe; do
        printf '\n[services.%s]\nenabled = true\n' "$app"
    done
    printf '\n[applications.rivet-proxyd]\nupstream-name = "%s"\n' "$MACHINE"
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
echo "adr-051-restore-proof: booting, control plane on $CONTROL_PORT, proxy on $PROXY_PORT"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --secrets "$WORK/secrets" \
    --document "$DOC" \
    --proxyd "$WORK/rivet-proxyd" \
    --app "restore-probe:$WORK/restore-probe:$WORK/restore-probe.toml:$WORK/restore-probe.header" \
    --app "plain-probe:$WORK/plain-probe:$WORK/plain-probe.toml:$WORK/plain-probe.header" \
    --trust "$WORK/trust" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$PROXY_PORT:443" \
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

# `cargo rivet <verb> …`. Sets RC; stdout and stderr land in files.
cr() {
    set +e
    "$CARGO_RIVET" "$@" >"$WORK/out" 2>"$WORK/err"
    RC=$?
    set -e
}
endpoint() {
    printf '%s\n' --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/operator" --pin "$GUEST_PIN"
}
rivet() {
    # shellcheck disable=SC2046
    cr run $(endpoint) -- "$@"
}
bytes_of() {
    od -An -v -tx1 "$1" | tr -d ' \n'
}
size_of() {
    wc -c <"$1" | tr -d ' '
}
pid_of() {
    rivet get status
    tr -d '\n' <"$WORK/out" | grep -o "\"name\":\"$1\"[^}]*" | grep -o '"pid":[0-9]*' | head -1 \
        | sed 's/.*://'
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

echo
echo "--- the table the supervisor derived ---"
line=$(grep -ao 'what this machine.s applications offer to back up[^}]*}' "$LOG" | tail -1 || true)
echo "  $line"
case $line in
    *'"declarations":2'*'"restores":1'*) ok "two declarations, one of which accepts a restore" ;;
    *) fail "the supervisor's line does not report 2 declarations and 1 restore: $line" ;;
esac
rivet get file /run/rivet/backups
if [ "$RC" -eq 0 ] && grep -q 'restore-probe probe restore api/v1/restore' "$WORK/out" \
    && ! grep -q 'plain-probe plain restore' "$WORK/out"; then
    ok "/run/rivet/backups carries restore-probe's restore line and no line for plain-probe"
else
    fail "the table read over the control plane is not what the manifests declare: $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- the TLS files, by the production path ---"
for app in rivet-proxyd restore-probe plain-probe; do
    # shellcheck disable=SC2046
    cr secrets "$app" $(endpoint) --authority "$WORK/ca" --name "$MACHINE" confirm
    [ "$RC" -eq 0 ] && ok "cargo rivet secrets $app" \
        || die "cargo rivet secrets $app exited $RC: $(cat "$WORK/out" "$WORK/err")"
done
for app in rivet-proxyd restore-probe plain-probe; do
    rivet apply service-restart "$app" confirm
    if [ "$RC" -ne 0 ]; then
        rivet apply service-start "$app" confirm
    fi
    echo "  $app: $(tr -d '\n' <"$WORK/out" | head -c 200) $(tr -d '\n' <"$WORK/err" | head -c 200)"
done

echo
echo "--- back up, over the control plane ---"
# shellcheck disable=SC2046
backup_into() {
    cr backup $(endpoint) --service restore-probe --into "$1" confirm
}
ATTEMPT=1
while :; do
    backup_into "$WORK/first"
    [ "$RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 30 ] && break
    rm -rf "$WORK/first"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
FIRST=$(ls "$WORK/first/restore-probe/"* 2>/dev/null | head -1 || true)
if [ "$RC" -eq 0 ] && [ -n "$FIRST" ] && [ "$(size_of "$FIRST")" -eq 2048 ]; then
    ok "the first backup is restore-probe's 2048-byte file, fetched over the control plane"
else
    die "the first backup did not arrive: $(cat "$WORK/out" "$WORK/err")"
fi
if grep -aq 'Backup of restore-probe relayed; the application answered 200' "$LOG"; then
    ok "rivet-mcpd journalled the relay to restore-probe on loopback"
else
    fail "no rivet-mcpd line says it relayed restore-probe's backup"
fi

# **No operator certificate within reach.** `HOME` names an empty directory, so
# there is no ~/.rivet/ca for anything to find; the fetch must not need one.
mkdir -p "$WORK/empty-home"
set +e
# shellcheck disable=SC2046
HOME="$WORK/empty-home" USERPROFILE="$WORK/empty-home" \
    "$CARGO_RIVET" backup $(endpoint) --service restore-probe --into "$WORK/no-cert" confirm \
    >"$WORK/out" 2>"$WORK/err"
RC=$?
set -e
NO_CERT=$(ls "$WORK/no-cert/restore-probe/"* 2>/dev/null | head -1 || true)
if [ "$RC" -eq 0 ] && [ -n "$NO_CERT" ] && [ "$(bytes_of "$NO_CERT")" = "$(bytes_of "$FIRST")" ]; then
    ok "with no operator certificate anywhere the tool could look, the backup still arrives whole"
else
    fail "the backup needed something from HOME: $(cat "$WORK/out" "$WORK/err")"
fi

for flag in --authority --name --port; do
    # shellcheck disable=SC2046
    cr backup $(endpoint) "$flag" x --into "$WORK/refused" confirm
    if [ "$RC" -ne 0 ] && grep -q -- "$flag is not an option" "$WORK/err"; then
        ok "backup refuses $flag by name"
    else
        fail "backup did not refuse $flag: $(cat "$WORK/out" "$WORK/err")"
    fi
done

echo
echo "--- the plan, and the refusals ---"
# shellcheck disable=SC2046
cr restore $(endpoint) --service restore-probe --from "$FIRST"
if [ "$RC" -eq 0 ] && grep -q 'Nothing was sent' "$WORK/out" \
    && grep -q "POST /restore/restore-probe on 127.0.0.1:$CONTROL_PORT" "$WORK/out" \
    && grep -q "POST /probe/api/v1/restore?confirm=true" "$WORK/out"; then
    ok "unconfirmed, it is a plan naming the control-plane route and the application's"
else
    fail "the unconfirmed restore was not a plan: $(cat "$WORK/out" "$WORK/err")"
fi

mkdir -p "$WORK/for-plain/plain-probe"
cp "$FIRST" "$WORK/for-plain/plain-probe/plain.sqlite"
# shellcheck disable=SC2046
cr restore $(endpoint) --service plain-probe --from "$WORK/for-plain/plain-probe/plain.sqlite" confirm
if [ "$RC" -ne 0 ] && grep -q 'NO RESTORE' "$WORK/err"; then
    ok "plain-probe, which declares no restore-path, is refused"
else
    fail "plain-probe was not refused: $(cat "$WORK/out" "$WORK/err")"
fi

mkdir -p "$WORK/wrong"
printf '{"not":"sqlite"}' >"$WORK/wrong/wrong.sqlite"
# shellcheck disable=SC2046
cr restore $(endpoint) --service restore-probe --from "$WORK/wrong/wrong.sqlite" confirm
if [ "$RC" -ne 0 ] && grep -q 'NOT a whole sqlite file' "$WORK/err"; then
    ok "a JSON file is refused as not the declared format"
else
    fail "the JSON file was not refused: $(cat "$WORK/out" "$WORK/err")"
fi

head -c 1024 "$FIRST" >"$WORK/wrong/short.sqlite"
# shellcheck disable=SC2046
cr restore $(endpoint) --service restore-probe --from "$WORK/wrong/short.sqlite" confirm
if [ "$RC" -ne 0 ] && grep -q "its own header says it is 2048 bytes and 1024 arrived" "$WORK/err"; then
    ok "the backup cut in half, magic intact, is refused at the format's own length claim"
else
    fail "the truncated file was not refused: $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- change the application's data ---"
if curl -sS --fail --cert "$WORK/ca/browser.crt" --key "$WORK/ca/browser.key" \
    --cacert "$WORK/ca/root.crt" --resolve "$MACHINE:$PROXY_PORT:127.0.0.1" \
    -X POST --data-binary '' "https://$MACHINE:$PROXY_PORT/probe/api/v1/mutate" >"$WORK/out" 2>"$WORK/err"; then
    ok "mutated: $(cat "$WORK/out")"
else
    fail "the mutate route did not answer: $(cat "$WORK/out" "$WORK/err")"
fi
backup_into "$WORK/mutated"
MUTATED=$(ls "$WORK/mutated/restore-probe/"* 2>/dev/null | head -1 || true)
if [ "$RC" -eq 0 ] && [ -n "$MUTATED" ] && [ "$(bytes_of "$MUTATED")" != "$(bytes_of "$FIRST")" ]; then
    ok "a second backup now differs from the first ($(size_of "$MUTATED") bytes)"
else
    fail "the data did not change: $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- a write another site caused is refused by rivet-proxyd (R-186 D1) ---"
# `curl` with the operator certificate, carrying the headers a browser adds
# when a page on another site causes the request. Page script cannot set
# either header, so this is the browser's side of the attack spelled by hand;
# no browser is driven here. `%{http_code}` is the proxy's answer, and the
# body is the first backup — the restore it would have been.
browser_post() {
    # $1 = path under the proxy, $2 = content type, $3 = file; the rest are -H.
    _path=$1 _type=$2 _file=$3
    shift 3
    set +e
    CODE=$(curl -sS --cert "$WORK/ca/browser.crt" --key "$WORK/ca/browser.key" \
        --cacert "$WORK/ca/root.crt" --resolve "$MACHINE:$PROXY_PORT:127.0.0.1" \
        -o "$WORK/out" -w '%{http_code}' -X POST -H "Content-Type: $_type" "$@" \
        --data-binary "@$_file" "https://$MACHINE:$PROXY_PORT$_path" 2>"$WORK/err")
    set -e
}
browser_post /probe/api/v1/restore?confirm=true text/plain "$FIRST" \
    -H 'Sec-Fetch-Site: cross-site' -H 'Origin: https://elsewhere.example'
if [ "$CODE" = 403 ] && grep -q 'another site' "$WORK/out"; then
    ok "Sec-Fetch-Site: cross-site, text/plain, to the restore route: 403 from the proxy"
else
    fail "the cross-site restore was not refused 403: code $CODE $(cat "$WORK/out" "$WORK/err")"
fi
browser_post /probe/api/v1/restore?confirm=true application/x-www-form-urlencoded "$FIRST" \
    -H 'Origin: https://elsewhere.example'
if [ "$CODE" = 403 ]; then
    ok "Origin from another site and no Sec-Fetch-Site: 403 from the proxy"
else
    fail "the Origin-only cross-site restore was not refused 403: code $CODE $(cat "$WORK/out" "$WORK/err")"
fi
backup_into "$WORK/after-attack"
AFTER_ATTACK=$(ls "$WORK/after-attack/restore-probe/"* 2>/dev/null | head -1 || true)
if [ "$RC" -eq 0 ] && [ -n "$AFTER_ATTACK" ] \
    && [ "$(bytes_of "$AFTER_ATTACK")" = "$(bytes_of "$MUTATED")" ]; then
    ok "after both, the application's data is still the mutated version — nothing was restored"
else
    fail "the data changed under a refused cross-site restore: $(cat "$WORK/out" "$WORK/err")"
fi
rivet get log latest all
if grep -aq 'refused a cross-site request that changes something' "$WORK/out" "$LOG"; then
    ok "rivet-proxyd journalled the refusal"
else
    fail "no journal record of the cross-site refusal in \`get log latest all\` or the console"
fi
# The application's own page: same-origin, and its own Origin, port included.
printf '' >"$WORK/empty"
browser_post /probe/api/v1/mutate application/octet-stream "$WORK/empty" \
    -H 'Sec-Fetch-Site: same-origin' -H "Origin: https://$MACHINE:$PROXY_PORT"
if [ "$CODE" = 200 ]; then
    ok "a same-origin POST, as the application's own page sends it, reaches the application"
else
    fail "a same-origin POST was not forwarded: code $CODE $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- restore ---"
BEFORE_PID=$(pid_of restore-probe)
# shellcheck disable=SC2046
cr restore $(endpoint) --service restore-probe --from "$FIRST" confirm
if [ "$RC" -eq 0 ] && grep -q 'restore-probe answered 200' "$WORK/out" \
    && grep -q '"restored":2048' "$WORK/out"; then
    ok "the application answered 200 and restored 2048 bytes"
else
    fail "the confirmed restore exited $RC: $(cat "$WORK/out" "$WORK/err")"
fi
if grep -aq 'Restore of restore-probe relayed; the application answered 200' "$LOG"; then
    ok "rivet-mcpd journalled relaying the restore to restore-probe on loopback"
else
    fail "no rivet-mcpd line says it relayed the restore"
fi
AFTER_PID=$(pid_of restore-probe)
if [ -n "$BEFORE_PID" ] && [ "$BEFORE_PID" = "$AFTER_PID" ]; then
    ok "restore-probe's pid is $AFTER_PID before and after — nothing was stopped"
else
    fail "restore-probe's pid changed or is unknown (${BEFORE_PID:-none} then ${AFTER_PID:-none})"
fi

backup_into "$WORK/after"
AFTER=$(ls "$WORK/after/restore-probe/"* 2>/dev/null | head -1 || true)
if [ "$RC" -eq 0 ] && [ -n "$AFTER" ] && [ "$(bytes_of "$AFTER")" = "$(bytes_of "$FIRST")" ]; then
    ok "a third backup is byte-identical to the first"
else
    fail "the data after the restore is not the first backup: $(cat "$WORK/out" "$WORK/err")"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-051-restore-proof: PASS — backed up, changed, restored, and identical"
else
    echo "adr-051-restore-proof: FAIL — the console log is at $LOG" >&2
    cp "$LOG" "$REPO/out/adr-051-restore-proof.console.log" 2>/dev/null || true
fi
exit "$VERDICT"
