#!/bin/sh
# tools/qemu/adr-060-proxy-proof.sh — **the image's own `rivet-proxyd` runs in
# user space under its own uid and ruleset, is handed its certificate from the
# card's staging bundle, and retires the application it used to be.**
#
# `ADR-060`, `.rulings/` R-209. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-060-proxy-proof.sh
#
# ## What it proves, over two boots
#
# **Boot 1 — a fresh machine whose document enables the proxy.** The proxy is
# packed at `/system/bin/rivet-proxyd` (run.sh's `--proxyd`), `/state` holds
# `staging/rivet-proxyd.txt` with the machine's certificate, key and fleet root,
# and the document names no `upstream-name`. It asserts:
#
# 1. `rivet-proxyd` is `started`, and the process's uid — the owner of its
#    `/proc/<pid>` directory — is non-zero and equal to the uid `/etc/passwd`
#    names for `rivet-proxyd`, which the supervisor derives with
#    `app_identity` (`ADR-060` §2);
# 2. the supervisor journalled the Landlock `ENFORCED` record for it, granting
#    `/system/bin/rivet-proxyd` read and execute;
# 3. it is bound on `0.0.0.0:443`;
# 4. the staged material was delivered into its data directory, owned by that
#    uid, and the staging copy is gone (`ADR-049`);
# 5. with no `upstream-name`, it took the first DNS name on its own certificate
#    (`ADR-060` §3.1);
# 6. the operator's browser certificate is served the index through it.
#
# **Boot 2 — the same machine, still holding the proxy APPLICATION it used to
# run.** `/state/app/rivet-proxyd` holds a `current` with a manifest and a
# program, and a `data` directory with the certificate, key and root owned by
# the proxy's uid; there is no staging bundle. It asserts:
#
# 7. the supervisor journalled the retirement once, at `info`, and nothing at
#    `error` about `rivet-proxyd`, and no name-collision refusal;
# 8. `current` is gone and `data/tls-cert` is still there;
# 9. the image's proxy runs, as the same uid, and serves the index from the
#    certificate the application left behind.
#
# ## What it does NOT settle
#
# No Raspberry Pi: the boot partition's courier into `/state/staging` is not
# emulated (`ADR-026` §4), so the bundle is placed on `/state` directly; a real
# update and trial boot are not run; no browser is driven. The throwaway fleet
# authority, certificates and identities are minted into this run's scratch
# directory and none of anybody's is read.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-060-proxy-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-060-proxy-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-060-proxy-proof: $1" >&2
    if [ -f "${LOG:-}" ]; then
        mkdir -p "$REPO/out" 2>/dev/null || true
        cp "$LOG" "$REPO/out/adr-060-proxy-proof.console.log" 2>/dev/null || true
    fi
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
PROXYD_PATH=$(constant "$REPO/rivet-core/src/service.rs" PROXYD_PATH)
PROXY_PORT=8443
GUEST=proxy-guest
SUFFIX=local
MACHINE=$GUEST.$SUFFIX
APP=rivet-proxyd
HOME_DIR=/state/app/$APP

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-060-proxy-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd \
    -p rivetos-proxyd ) || die "the guest's binaries did not build"
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision ) \
    || die "the workstation tools did not build"
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet
PROVISION=$TARGET_DIR/release/rivet-provision

# Stripped, as `tools/card/build.sh` strips every program it puts on a card.
STRIP="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-strip"
[ -x "$STRIP" ] || die "no $STRIP to strip rivet-proxyd with"
"$STRIP" -o "$WORK/rivet-proxyd" "$OUT/rivet-proxyd" || die "rivet-proxyd could not be stripped"

# ---------------------------------------------------------------------------
# Throwaway identities, and a throwaway fleet authority
# ---------------------------------------------------------------------------
echo "adr-060-proxy-proof: minting THROWAWAY identities and an authority in $WORK"
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

HOME="$WORK/home" "$PROVISION" ca "$SUFFIX" proxy-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no authority"; }
HOME="$WORK/home" "$PROVISION" ca server "$GUEST" "$WORK/ca" \
    >>"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no server pair"; }
CERT=$WORK/ca/server-$MACHINE.crt
KEY=$WORK/ca/server-$MACHINE.key
ROOT=$WORK/ca/root.crt
[ -f "$CERT" ] && [ -f "$KEY" ] && [ -f "$ROOT" ] || die "rivet-provision did not write the pair"

# ---------------------------------------------------------------------------
# Boot 1's /state: a staging bundle for the proxy, as PID 1 leaves it
# ---------------------------------------------------------------------------
# `rivet_core::staging`'s format: a version line, then one
# `<application> <relative/path> <hex of the contents>` per file.
hex_of() {
    od -An -v -tx1 "$1" | tr -d ' \n'
}
STATE1=$WORK/state1
mkdir -p "$STATE1/staging"
{
    printf '# staged by tools/qemu/adr-060-proxy-proof.sh\nversion = 1\n'
    printf '%s tls-cert %s\n' "$APP" "$(hex_of "$CERT")"
    printf '%s tls-key %s\n' "$APP" "$(hex_of "$KEY")"
    printf '%s tls-root %s\n' "$APP" "$(hex_of "$ROOT")"
} >"$STATE1/staging/$APP.txt"
chmod 600 "$STATE1/staging/$APP.txt"

# **No `upstream-name`**: the proxy takes the first DNS name on its own
# certificate, which `ca server` issued for `$MACHINE`.
DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[services.%s]\nenabled = true\n' "$APP"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

QEMU=
stop_guest() {
    if [ -n "$QEMU" ]; then
        kill "$QEMU" 2>/dev/null || true
        wait "$QEMU" 2>/dev/null || true
        QEMU=
    fi
}
trap stop_guest EXIT

# `boot STATE-DIR LOG [run.sh arguments…]`
boot() {
    _state=$1 _log=$2
    shift 2
    "$HERE/run.sh" "$OUT/rivet-initd" \
        --supervisor "$OUT/rivet-sysd" \
        --cli "$OUT/rivet" \
        --netd "$OUT/rivet-netd" \
        --mcpd "$OUT/rivet-mcpd" \
        --proxyd "$WORK/rivet-proxyd" \
        --secrets "$WORK/secrets" \
        --document "$DOC" \
        --state "$_state" \
        "$@" \
        --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$PROXY_PORT:443" \
        >"$_log" 2>&1 </dev/null &
    QEMU=$!
    LOG=$_log
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
}

cr() {
    set +e
    "$CARGO_RIVET" "$@" >"$WORK/out" 2>"$WORK/err"
    RC=$?
    set -e
}
rivet() {
    cr run --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/operator" --pin "$GUEST_PIN" -- "$@"
}

# The proxy's row in `get status`, and its pid.
row() {
    tr -d '\n' <"$WORK/out" | grep -o "\"name\":\"$APP\"[^}]*" | head -1
}
# Wait for `rivet-proxyd` to be `started`; sets PID.
wait_started() {
    ATTEMPT=1
    PID=
    while :; do
        rivet get status
        if [ "$RC" -eq 0 ]; then
            case $(row) in
                *'"state":"started"'*)
                    PID=$(row | grep -o '"pid":[0-9]*' | head -1 | sed 's/.*://')
                    [ -n "$PID" ] && return 0
                    ;;
            esac
        fi
        [ "$ATTEMPT" -ge 40 ] && return 1
        ATTEMPT=$((ATTEMPT + 1))
        sleep 2
    done
}

# `uid_of PATH` — the `uid` `get file` reports, or empty.
uid_of() {
    rivet get file "$1"
    tr -d '\n' <"$WORK/out" | grep -oE '"uid": ?[0-9]+' | head -1 | grep -oE '[0-9]+$' || true
}
present() {
    rivet get file "$1"
    tr -d '\n' <"$WORK/out" | grep -qE '"present": ?true'
}

https_index() {
    set +e
    CODE=$(curl -sS --max-time 20 --cert "$WORK/ca/browser.crt" --key "$WORK/ca/browser.key" \
        --cacert "$ROOT" --resolve "$MACHINE:$PROXY_PORT:127.0.0.1" \
        -o "$WORK/index" -w '%{http_code}' "https://$MACHINE:$PROXY_PORT/" 2>"$WORK/curl-err")
    set -e
}

# `the_proxy_runs_as_its_own_uid WHICH` — assertions 1 and 9's shared half.
the_proxy_runs_as_its_own_uid() {
    rivet get file /etc/passwd
    PASSWD_UID=$(tr -d '\n' <"$WORK/out" | grep -oE "$APP:x:[0-9]+:" | head -1 \
        | cut -d: -f3 || true)
    PROC_UID=$(uid_of "/proc/$PID")
    if [ -n "$PASSWD_UID" ] && [ "$PASSWD_UID" != 0 ] && [ "$PROC_UID" = "$PASSWD_UID" ]; then
        ok "$1: $APP (pid $PID) runs as uid $PROC_UID, the uid /etc/passwd names for it, not root"
    else
        fail "$1: $APP's process uid '$PROC_UID' is not the non-zero uid /etc/passwd names \
('$PASSWD_UID')"
    fi
}

# ===========================================================================
# BOOT 1
# ===========================================================================
echo
echo "adr-060-proxy-proof: BOOT 1 — a fresh machine, the proxy staged for"
boot "$STATE1" "$WORK/console-1.log"
wait_started || { tail -80 "$LOG" >&2; die "BOOT 1: $APP never reached started: $(row)"; }
ok "BOOT 1: $APP is started, pid $PID"

the_proxy_runs_as_its_own_uid "BOOT 1"
BOOT1_UID=$PASSWD_UID

rivet get log latest all
cat "$WORK/out" >"$WORK/journal-1"
if grep -ah "\"svc\":\"$APP\"" "$WORK/journal-1" "$LOG" | grep -a "ENFORCED this application is confined" \
    | grep -aqF "$PROXYD_PATH (read+execute)"; then
    ok "BOOT 1: the supervisor journalled ENFORCED for $APP, granting $PROXYD_PATH read+execute"
else
    fail "BOOT 1: no Landlock ENFORCED record granting $PROXYD_PATH to $APP"
fi

bound=$(grep -aoh "\"svc\":\"$APP\"[^}]*tcp and udp sockets are bound[^}]*" \
    "$WORK/journal-1" "$LOG" | grep -o '"bound":"[^"]*"' | tail -1 || true)
if [ "$bound" = '"bound":"0.0.0.0:443"' ]; then
    ok "BOOT 1: $APP is bound on every address: $bound"
else
    fail "BOOT 1: $APP's bind record is not 0.0.0.0:443: '$bound'"
fi

if grep -ah "\"svc_name\":\"$APP\"" "$WORK/journal-1" "$LOG" \
    | grep -aq "handed the material the card carried"; then
    ok "BOOT 1: the supervisor handed $APP its staged material"
else
    fail "BOOT 1: no record of $APP being handed its staged material"
fi
CERT_UID=$(uid_of "$HOME_DIR/data/tls-cert")
if [ -n "$CERT_UID" ] && [ "$CERT_UID" = "$BOOT1_UID" ]; then
    ok "BOOT 1: $HOME_DIR/data/tls-cert is owned by uid $CERT_UID"
else
    fail "BOOT 1: $HOME_DIR/data/tls-cert is owned by '$CERT_UID', not $APP's uid '$BOOT1_UID'"
fi
if present "/state/staging/$APP.txt"; then
    fail "BOOT 1: the staging copy of $APP's key is still on /state"
else
    ok "BOOT 1: the staging copy is gone"
fi

if grep -ah "\"svc\":\"$APP\"" "$WORK/journal-1" "$LOG" \
    | grep -a "verifying the applications behind this proxy as" \
    | grep -aq "first DNS name on this proxy's certificate"; then
    ok "BOOT 1: with no upstream-name, $APP took the first DNS name on its certificate"
else
    fail "BOOT 1: no record of $APP taking its upstream name from its certificate"
fi

ATTEMPT=1
while :; do
    https_index
    [ "$CODE" = 200 ] && break
    [ "$ATTEMPT" -ge 15 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$CODE" = 200 ] && grep -q 'Authenticated as' "$WORK/index"; then
    ok "BOOT 1: the operator's browser certificate is served the index through $APP"
else
    fail "BOOT 1: the index was not served: $CODE $(cat "$WORK/index" "$WORK/curl-err" 2>/dev/null)"
fi

stop_guest

# ===========================================================================
# BOOT 2
# ===========================================================================
# `/state` holding the proxy APPLICATION this machine used to run: its data,
# owned by its uid, and — through run.sh's `--app` — a `current` with a
# manifest and a program. No staging bundle: the data is all it has.
STATE2=$WORK/state2
mkdir -p "$STATE2/app/$APP/data"
cp "$CERT" "$STATE2/app/$APP/data/tls-cert"
cp "$KEY" "$STATE2/app/$APP/data/tls-key"
cp "$ROOT" "$STATE2/app/$APP/data/tls-root"
chmod 700 "$STATE2/app/$APP/data"
chmod 600 "$STATE2/app/$APP/data/tls-cert" "$STATE2/app/$APP/data/tls-key" \
    "$STATE2/app/$APP/data/tls-root"
chown -R "$BOOT1_UID:$BOOT1_UID" "$STATE2/app/$APP/data" \
    || die "could not give the old application's data to uid $BOOT1_UID"
{
    printf 'kind = "daemon"\nrestart = "on-failure"\nready = "none"\nnetwork = "none"\n'
    printf 'state = "private"\nlisten = ["tcp:443"]\nprovides = []\nconnects = []\n'
    printf 'library = []\ndevices = []\nsettings = ["upstream-name"]\n'
} >"$WORK/old-proxy.toml"

echo
echo "adr-060-proxy-proof: BOOT 2 — the same machine, still holding the proxy application"
boot "$STATE2" "$WORK/console-2.log" --app "$APP:$WORK/rivet-proxyd:$WORK/old-proxy.toml"
wait_started || { tail -80 "$LOG" >&2; die "BOOT 2: $APP never reached started: $(row)"; }
ok "BOOT 2: $APP is started, pid $PID"

rivet get log latest all
cat "$WORK/out" >"$WORK/journal-2"
# The retirement runs before discovery, early enough in the boot that its
# record may reach the console and not the journal `get log` reads, so both are
# searched and a record seen in both is counted once, by its correlation.
RETIRED=$(grep -ah "now provides was removed" "$WORK/journal-2" "$LOG" \
    | grep -a "\"svc_name\":\"$APP\"" || true)
RETIRED_COUNT=$(printf '%s\n' "$RETIRED" | grep -o '"corr":"[^"]*"' | sort -u | grep -c . || true)
if [ "$RETIRED_COUNT" = 1 ] && printf '%s' "$RETIRED" | grep -q '"sev":"info"'; then
    ok "BOOT 2: the retirement of the $APP application was journalled once, at info"
else
    fail "BOOT 2: the retirement was journalled $RETIRED_COUNT time(s): $RETIRED"
fi
if grep -ah '"sev":"error"' "$WORK/journal-2" "$LOG" | grep -aq "$APP"; then
    fail "BOOT 2: an error record names $APP: \
$(grep -ah '"sev":"error"' "$WORK/journal-2" "$LOG" | grep -a "$APP" | head -3)"
else
    ok "BOOT 2: no error record names $APP"
fi
if grep -aq "same name as a service this machine already defines" "$WORK/journal-2" "$LOG"; then
    fail "BOOT 2: discovery refused a name collision — the application was not retired first"
else
    ok "BOOT 2: no name collision was refused"
fi

if present "$HOME_DIR/current"; then
    fail "BOOT 2: $HOME_DIR/current is still there"
else
    ok "BOOT 2: $HOME_DIR/current is gone"
fi
if present "$HOME_DIR/data/tls-cert"; then
    ok "BOOT 2: $HOME_DIR/data/tls-cert was kept"
else
    fail "BOOT 2: the application's data was not kept"
fi

the_proxy_runs_as_its_own_uid "BOOT 2"
if [ "$PASSWD_UID" != "$BOOT1_UID" ]; then
    fail "BOOT 2: $APP's uid moved between boots: $BOOT1_UID then $PASSWD_UID"
fi

ATTEMPT=1
while :; do
    https_index
    [ "$CODE" = 200 ] && break
    [ "$ATTEMPT" -ge 15 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$CODE" = 200 ] && grep -q 'Authenticated as' "$WORK/index"; then
    ok "BOOT 2: the image's $APP serves the index from the certificate the application left"
else
    fail "BOOT 2: the index was not served: $CODE $(cat "$WORK/index" "$WORK/curl-err" 2>/dev/null)"
fi

stop_guest

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-060-proxy-proof: PASS — the image's proxy runs as its own uid under a ruleset, is \
handed its certificate, and retires the application it used to be"
else
    echo "adr-060-proxy-proof: FAIL — see the lines above" >&2
    mkdir -p "$REPO/out" 2>/dev/null || true
    cp "$WORK/console-1.log" "$REPO/out/adr-060-proxy-proof.console-1.log" 2>/dev/null || true
    cp "$WORK/console-2.log" "$REPO/out/adr-060-proxy-proof.console-2.log" 2>/dev/null || true
fi
exit "$VERDICT"
