#!/bin/sh
# tools/qemu/adr-046-backend-port-proof.sh — **who can reach an application
# behind `rivet-proxyd`: through the proxy's gate, on the application's routed
# port, and on a port it declared `:direct`.**
#
# `ADR-046` §4.1, §4.5 and §7, `.rulings/` R-187 D1 and R-188. Run it in the
# container:
#
#   tools/container/exec.sh sh tools/qemu/adr-046-backend-port-proof.sh
#
# ## What it proves
#
# 1. **The operator gate, through the proxy.** The operator's browser
#    certificate is served the index; another fleet machine's front-door
#    certificate is refused the index and a gated route, `403` naming its host,
#    with a journal record.
# 2. **The same operations still work through the proxy, and its guards hold.**
#    The operator reads, changes and restores the application's data through
#    `rivet-proxyd`; another machine's certificate cannot write it there; a
#    cross-site `text/plain` restore under the operator's certificate is
#    refused `403` and changes nothing.
# 3. **The routed port refuses the network** (R-188). The supervisor binds a
#    proxied application's port on `127.0.0.1` (`cmd/rivet-sysd/src/kernel.rs`,
#    `open_listeners`, by `Socket::bind_address`) and journals the address.
#    QEMU's forward reaches the guest's non-loopback address, which is what a
#    LAN peer reaches, so a request there completes nothing — not another
#    machine's read or write, and not the cross-site restore — and the data is
#    unchanged by each attempt.
# 4. **A `:direct` port stays reachable.** The same probe declares a second
#    port `tcp:8081:direct`, and the request refused on the routed port is
#    answered there: the difference is the bind address and not the
#    certificate, the forward or TLS.
#
# ## What is on the guest
#
# The control plane (`rivet-mcpd`, `rivet-netd`), `rivet-proxyd` — part of the
# image (`ADR-060`, run.sh's `--proxyd`) — holding 443 (forwarded from 8443)
# and not behind itself, so on every address, and
# `tools/qemu/restore-probe` installed as `restore-probe` behind
# `proxy-path = "probe"`, declaring `tcp:8080` (forwarded from 18080) and
# `tcp:8081:direct` (forwarded from 18081). The fleet authority, the operator's
# `browser` pair, the guest's server pair and a second machine's pair
# (`rivet-02`) are minted by `rivet-provision` into this run's scratch
# directory. Every key is THROWAWAY and none of anybody's is read.
#
# ## What it does NOT settle
#
# No Raspberry Pi and no house network: the guest is reached through SLIRP's
# forward, which proves what the guest's non-loopback address answers, not what
# a LAN peer's route to it is. `curl` sends the browser's headers; no browser is
# driven. A process on the guest itself still reaches the routed port on
# loopback, which is the design (`ADR-046` §4.2) and not exercised here.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-046-backend-port-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-046-backend-port-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-046-backend-port-proof: $1" >&2
    exit 1
}
ok() {
    echo "  OK   $1"
}

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
APP_GUEST_PORT=8080
APP_PORT=18080
DIRECT_GUEST_PORT=8081
DIRECT_PORT=18081
GUEST=backend-guest
OTHER=rivet-02
SUFFIX=local
MACHINE=$GUEST.$SUFFIX

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-046-backend-port-proof: building the guest's binaries for $TARGET"
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
echo "adr-046-backend-port-proof: minting THROWAWAY identities and an authority in $WORK"
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

HOME="$WORK/home" "$PROVISION" ca "$SUFFIX" port-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no authority"; }
for host in "$GUEST" "$OTHER"; do
    HOME="$WORK/home" "$PROVISION" ca server "$host" "$WORK/ca" \
        >>"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no $host pair"; }
    [ -f "$WORK/ca/server-$host.$SUFFIX.crt" ] || die "rivet-provision did not write $host's pair"
done

# ---------------------------------------------------------------------------
# The one installed application, signed by the fixture key
# ---------------------------------------------------------------------------
# Canonical text in `KEYS` order, as `Manifest::render` writes it. `$1` is the
# whole `listen` list's contents.
manifest_head() {
    printf 'kind = "daemon"\nrestart = "on-failure"\nready = "none"\nnetwork = "none"\n'
    printf 'state = "private"\nlisten = [%s]\nprovides = []\nconnects = []\n' "$1"
    printf 'library = []\ndevices = []\n'
}
{
    manifest_head "\"tcp:$APP_GUEST_PORT\", \"tcp:$DIRECT_GUEST_PORT:direct\""
    printf 'proxy-path = "probe"\nbackup-path = "api/v1/backup"\nbackup-format = "sqlite"\n'
    printf 'restore-path = "api/v1/restore"\n'
} >"$WORK/restore-probe.toml"

# The proxy is part of the image (`ADR-060`), packed at /system/bin by run.sh's
# `--proxyd` and stripped as `tools/card/build.sh` strips it.
STRIP="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-strip"
[ -x "$STRIP" ] || die "no $STRIP to strip rivet-proxyd with"
"$STRIP" -o "$WORK/rivet-proxyd" "$OUT/rivet-proxyd" || die "rivet-proxyd could not be stripped"
cp "$PROBE" "$WORK/restore-probe"
"$SIGNER" restore-probe "$WORK/restore-probe" "$WORK/restore-probe.toml" \
    "$WORK/restore-probe.header" "$WORK/trust" >/dev/null || die "restore-probe could not be signed"

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for app in rivet-proxyd restore-probe; do
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
echo "adr-046-backend-port-proof: booting; proxy on $PROXY_PORT, the routed port on $APP_PORT, \
the direct port on $DIRECT_PORT"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --secrets "$WORK/secrets" \
    --document "$DOC" \
    --proxyd "$WORK/rivet-proxyd" \
    --app "restore-probe:$WORK/restore-probe:$WORK/restore-probe.toml:$WORK/restore-probe.header" \
    --trust "$WORK/trust" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$PROXY_PORT:443" \
    --forward-port "$APP_PORT:$APP_GUEST_PORT" --forward-port "$DIRECT_PORT:$DIRECT_GUEST_PORT" \
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
echo "--- the TLS files, by the production path ---"
for app in rivet-proxyd restore-probe; do
    # shellcheck disable=SC2046
    cr secrets "$app" $(endpoint) --authority "$WORK/ca" --name "$MACHINE" confirm
    [ "$RC" -eq 0 ] && ok "cargo rivet secrets $app" \
        || die "cargo rivet secrets $app exited $RC: $(cat "$WORK/out" "$WORK/err")"
done
for app in rivet-proxyd restore-probe; do
    rivet apply service-restart "$app" confirm
    if [ "$RC" -ne 0 ]; then
        rivet apply service-start "$app" confirm
    fi
done

# `https PORT PATH CERT-STEM [curl arguments…]` — one request presenting
# `$WORK/ca/<stem>.crt` (or nothing, for stem `none`). Sets CODE to the HTTP
# status, or `000` when no response arrived; the body lands in $WORK/out.
https() {
    _port=$1 _path=$2 _stem=$3
    shift 3
    set +e
    if [ "$_stem" = none ]; then
        CODE=$(curl -sS --max-time 20 --cacert "$WORK/ca/root.crt" \
            --resolve "$MACHINE:$_port:127.0.0.1" -o "$WORK/out" -w '%{http_code}' "$@" \
            "https://$MACHINE:$_port$_path" 2>"$WORK/err")
    else
        CODE=$(curl -sS --max-time 20 --cert "$WORK/ca/$_stem.crt" --key "$WORK/ca/$_stem.key" \
            --cacert "$WORK/ca/root.crt" --resolve "$MACHINE:$_port:127.0.0.1" \
            -o "$WORK/out" -w '%{http_code}' "$@" "https://$MACHINE:$_port$_path" 2>"$WORK/err")
    fi
    set -e
}
MEMBER=server-$OTHER.$SUFFIX

# The application answering through the proxy is the readiness signal.
ATTEMPT=1
while :; do
    https "$PROXY_PORT" /probe/api/v1/backup browser
    [ "$CODE" = 200 ] && cp "$WORK/out" "$WORK/first.sqlite" && break
    [ "$ATTEMPT" -ge 30 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
[ "$CODE" = 200 ] || die "the application never answered through the proxy: $CODE $(cat "$WORK/err")"

size_of() {
    wc -c <"$1" | tr -d ' '
}
# The data's size, read through the proxy with the operator's certificate:
# 2048 bytes is the first version and 3072 the mutated one.
backup_size() {
    https "$PROXY_PORT" /probe/api/v1/backup browser
    if [ "$CODE" = 200 ]; then size_of "$WORK/out"; else echo "no-backup-$CODE"; fi
}

echo
echo "--- where the supervisor bound the ports (R-188) ---"
rivet get log latest all
cat "$WORK/out" >"$WORK/journal"
bound=$(grep -ao "\"svc\":\"restore-probe\"[^}]*tcp and udp sockets are bound[^}]*" \
    "$WORK/journal" "$LOG" | grep -o '"bound":"[^"]*"' | tail -1 || true)
if [ "$bound" = "\"bound\":\"127.0.0.1:$APP_GUEST_PORT,0.0.0.0:$DIRECT_GUEST_PORT\"" ]; then
    ok "restore-probe's journal says $bound — the routed port on loopback, the direct one everywhere"
else
    fail "restore-probe's bind record is not loopback then every address: '$bound'"
fi
proxy_bound=$(grep -ao "\"svc\":\"rivet-proxyd\"[^}]*tcp and udp sockets are bound[^}]*" \
    "$WORK/journal" "$LOG" | grep -o '"bound":"[^"]*"' | tail -1 || true)
if [ "$proxy_bound" = '"bound":"0.0.0.0:443"' ]; then
    ok "rivet-proxyd, which is not behind itself, is bound on every address: $proxy_bound"
else
    fail "rivet-proxyd's bind record is not 0.0.0.0:443: '$proxy_bound'"
fi

echo
echo "--- the operator gate, through the proxy (R-187 D1) ---"
https "$PROXY_PORT" / browser
if [ "$CODE" = 200 ] && grep -q 'Authenticated as' "$WORK/out"; then
    ok "the operator's browser certificate is served the index"
else
    fail "the operator was not served the index: $CODE $(cat "$WORK/out" "$WORK/err")"
fi
https "$PROXY_PORT" / "$MEMBER"
if [ "$CODE" = 403 ] && grep -q "$OTHER.$SUFFIX" "$WORK/out"; then
    ok "$OTHER's front-door certificate is refused the index, 403 naming $OTHER.$SUFFIX"
else
    fail "$OTHER's certificate was not refused the index: $CODE $(cat "$WORK/out" "$WORK/err")"
fi
https "$PROXY_PORT" /probe/api/v1/backup "$MEMBER"
if [ "$CODE" = 403 ]; then
    ok "$OTHER's front-door certificate is refused the gated route, 403"
else
    fail "$OTHER's certificate was not refused the gated route: $CODE $(cat "$WORK/out")"
fi
rivet get log latest all
if grep -aq "not the operator's at the operator gate" "$WORK/out" "$LOG"; then
    ok "rivet-proxyd journalled the refusal"
else
    fail "no journal record of the refusal in \`get log latest all\` or the console"
fi

echo
echo "--- the same operations, through the proxy ---"
BEFORE=$(backup_size)
https "$PROXY_PORT" /probe/api/v1/mutate "$MEMBER" -X POST --data-binary ''
AFTER=$(backup_size)
if [ "$CODE" = 403 ] && [ "$BEFORE" = "$AFTER" ]; then
    ok "$OTHER's certificate cannot write through the proxy (403; $BEFORE then $AFTER bytes)"
else
    fail "$OTHER's write through the proxy was not refused: $CODE; $BEFORE then $AFTER"
fi

https "$PROXY_PORT" /probe/api/v1/mutate browser -X POST --data-binary ''
MUTATED=$(backup_size)
if [ "$CODE" = 200 ] && [ "$MUTATED" = 3072 ]; then
    ok "the operator changes the data through the proxy (200; now $MUTATED bytes)"
else
    fail "the operator's mutate through the proxy did not land: $CODE; now $MUTATED"
fi

https "$PROXY_PORT" '/probe/api/v1/restore?confirm=true' browser -X POST \
    -H 'Content-Type: text/plain' -H 'Sec-Fetch-Site: cross-site' \
    -H 'Origin: https://elsewhere.example' --data-binary "@$WORK/first.sqlite"
AFTER=$(backup_size)
if [ "$CODE" = 403 ] && [ "$AFTER" = "$MUTATED" ]; then
    ok "a cross-site text/plain restore with the operator's certificate is refused by the proxy \
(403; still $AFTER bytes)"
else
    fail "the cross-site restore through the proxy was not refused: $CODE; now $AFTER bytes"
fi

echo
echo "--- the routed port, not through the proxy (ADR-046 §7, R-188) ---"
https "$APP_PORT" /api/v1/backup "$MEMBER"
if [ "$CODE" = 000 ]; then
    ok "$OTHER's front-door certificate cannot read the application on its routed port \
(no response: $(tr -d '\n' <"$WORK/err" | head -c 120))"
else
    fail "$OTHER's certificate reached the routed port from outside the guest: $CODE \
($(size_of "$WORK/out") bytes)"
fi

BEFORE=$(backup_size)
https "$APP_PORT" /api/v1/mutate "$MEMBER" -X POST --data-binary ''
AFTER=$(backup_size)
if [ "$CODE" = 000 ] && [ "$BEFORE" = "$AFTER" ]; then
    ok "$OTHER's front-door certificate cannot write it there (no response; $BEFORE then $AFTER bytes)"
else
    fail "$OTHER's certificate wrote the routed port: $CODE; backup $BEFORE bytes before, $AFTER after"
fi

https "$APP_PORT" '/api/v1/restore?confirm=true' browser -X POST \
    -H 'Content-Type: text/plain' -H 'Sec-Fetch-Site: cross-site' \
    -H 'Origin: https://elsewhere.example' --data-binary "@$WORK/first.sqlite"
AFTER=$(backup_size)
if [ "$CODE" = 000 ] && [ "$AFTER" = "$MUTATED" ]; then
    ok "a cross-site restore with the operator's certificate completes nothing on the routed port \
(no response; still $AFTER bytes)"
else
    fail "a cross-site restore reached the routed port: $CODE; backup now $AFTER bytes"
fi

echo
echo "--- the :direct port, not through the proxy ---"
https "$DIRECT_PORT" /api/v1/backup "$MEMBER"
if [ "$CODE" = 200 ] && [ "$(size_of "$WORK/out")" = "$MUTATED" ]; then
    ok "the request refused on the routed port is answered on tcp:$DIRECT_GUEST_PORT:direct \
(200, $MUTATED bytes) — reach is the difference, not the certificate or the forward"
else
    fail "the :direct port did not answer from outside the guest: $CODE $(cat "$WORK/err")"
fi

echo
echo "--- the operator restores through the proxy ---"
https "$PROXY_PORT" '/probe/api/v1/restore?confirm=true' browser -X POST \
    -H 'Content-Type: application/octet-stream' --data-binary "@$WORK/first.sqlite"
RESTORED=$(backup_size)
if [ "$CODE" = 200 ] && [ "$RESTORED" = 2048 ]; then
    ok "the operator's restore through the proxy is served (200; back to $RESTORED bytes)"
else
    fail "the operator's restore through the proxy did not land: $CODE; now $RESTORED bytes"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-046-backend-port-proof: PASS — the gate holds through the proxy, the routed port \
refuses the network, and a :direct port stays reachable"
else
    echo "adr-046-backend-port-proof: FAIL — see the lines above" >&2
    mkdir -p "$REPO/out" 2>/dev/null || true
    cp "$LOG" "$REPO/out/adr-046-backend-port-proof.console.log" 2>/dev/null || true
fi
exit "$VERDICT"
