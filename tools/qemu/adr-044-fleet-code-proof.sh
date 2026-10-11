#!/bin/sh
# tools/qemu/adr-044-fleet-code-proof.sh — **fleet as code, run by a CONFINED
# rivet-fleetd on a booted guest, its git asked of the guest's own control
# plane.**
#
# `ADR-044` §11, `.rulings/` R-187 D2. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-044-fleet-code-proof.sh
#
# ## What is on the guest
#
# One aarch64 guest that is its own fleet server: the control plane
# (`rivet-mcpd`, `rivet-netd`), the vendored `git` and `git-http-backend`, a
# document declaring `[repos.machines]`, and the real `rivet-fleetd`, stripped,
# signed with the fixture key and installed with `--app`, ENABLED, so the
# supervisor starts it under its own Landlock ruleset. Its certificates, its
# operator identity and a one-record register naming the guest as
# `role = "fleet-server"` are staged under `/state/staging` exactly as a card
# stages them, so the supervisor hands them over with the application's uid.
# `fleet-repository = "machines"` and a three-second poll turn the poller on.
#
# ## What it asserts
#
# 1. **rivet-fleetd is confined** — the supervisor's `ENFORCED` record names
#    it — and serves 4443 to the operator's browser certificate.
# 2. **The poller adopts the guest from the repository**: the guest's own
#    document is pushed verbatim into `machines.git` with a stock git over the
#    control plane, and the confined poller records the pushed commit as the
#    guest's baseline, read off the machine with `get file`. Every git
#    question it asked is in `rivet-mcpd`'s journal under the fleet server's
#    own operator key.
# 3. **A commit through `POST /api/commit/<device>` converges the machine**:
#    the operator's certificate commits a renamed document, the commit is on
#    the repository's `main` (read with `ls-remote`), the poller sees the new
#    head and writes the guest, and the guest's own `get desired-state`
#    carries the new hostname.
# 4. **A stale compare-and-swap is refused, both ways**: the same POST again,
#    whose `expected` document is now stale, is refused by rivet-fleetd; and
#    `POST /fleet-as-code/commit` sent straight to 7777 with a stale
#    `expected-head` is refused `409 stale-head`. `main` does not move.
# 5. **A fleet member is refused, both ways**: another machine's front-door
#    certificate is refused at 4443 (`403`, `-32003`), and a control-plane
#    identity not in the guest's `peers` completes no request on 7777.
#
# ## What it does NOT settle
#
# `ADR-026` §4: no Raspberry Pi, no SD card, QEMU's SLIRP forwarding rather than
# `rivet-netd`'s path, and `/state` is the initramfs tmpfs rather than ext4.
# The fleet server converges ITSELF, the only machine one guest holds; a
# second machine's convergence, drift refusal and override are
# `tools/qemu/adr-044-proof.sh`'s. Every key is THROWAWAY, minted into this
# run's working directory, and the application is signed by the fixture key.
#
# The guest has **no `/dev/null` and no `/dev/kmsg`** (`AGENTS.md` §10.5).
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release
HOST_OUT=$TARGET_DIR/release

BR=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}
CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$BR/host/bin/aarch64-linux-gcc}"
export CC_aarch64_unknown_linux_musl

STAGE=
note() { echo "adr-044-fleet-code-proof: $*"; }
fail() {
    echo >&2
    echo "adr-044-fleet-code-proof: FAIL${STAGE:+ [$STAGE]}: $*" >&2
    exit 1
}

command -v qemu-system-aarch64 >/dev/null 2>&1 || fail "qemu-system-aarch64 is not on PATH"
command -v git >/dev/null 2>&1 || fail "no host git on PATH"
command -v openssl >/dev/null 2>&1 || fail "no openssl on PATH"
command -v curl >/dev/null 2>&1 || fail "no curl on PATH"
command -v od >/dev/null 2>&1 || fail "no od on PATH"
[ -x "$CC_aarch64_unknown_linux_musl" ] || fail "no cross C compiler at $CC_aarch64_unknown_linux_musl"
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi
[ -n "${RIVET_KERNEL:-}" ] || fail "no RIVET_KERNEL and no /br/images/Image"
GUEST_GIT=$BR/target/usr/bin/git
GUEST_HTTP_BACKEND=$BR/target/usr/libexec/git-core/git-http-backend
[ -f "$GUEST_GIT" ] || fail "no vendored git at $GUEST_GIT"
[ -f "$GUEST_HTTP_BACKEND" ] || fail "no git-http-backend at $GUEST_HTTP_BACKEND"

# --- constants read out of the crates that own them -------------------------

constant() {
    _value=$(sed -n "s/^pub const $2: *&str *= *\"\([^\"]*\)\".*/\1/p" "$1" | head -1)
    [ -n "$_value" ] || fail "cannot read $2 out of $1 — this harness agrees with that crate and will not guess"
    printf '%s\n' "$_value"
}
CONTROL_PORT=$(grep -oE 'DEFAULT_PORT: u16 = [0-9]+' "$REPO/rivet-config/src/control_plane.rs" | grep -oE '[0-9]+$')
[ -n "$CONTROL_PORT" ] || fail "could not read rivet_config::control_plane::DEFAULT_PORT"
MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
STAGING=$(constant "$REPO/rivet-core/src/state.rs" STAGING)
BUILT_IN=$(constant "$REPO/rivet-core/src/service.rs" BUILT_IN_VOLUME)
SERVER_ROLE=$(constant "$REPO/rivet-core/src/fleet_as_code.rs" SERVER_ROLE)
MACHINES_DIR=$(constant "$REPO/rivet-core/src/fleet_as_code.rs" MACHINES_DIR)
CODE_DIR=$(constant "$REPO/cmd/rivet-fleetd/src/code.rs" DIR)
BASELINES=$(constant "$REPO/cmd/rivet-fleetd/src/code.rs" BASELINES)
FLEETD=rivet-fleetd
DATA_DIR=$BUILT_IN/app/$FLEETD/data
FLEET_PORT=14443
SUFFIX=local
GUEST=fleet-guest
MEMBER=rivet-02
REPOSITORY=machines
DEVICE_ID=0011223344556677

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr044-code.XXXXXX") || fail "no working directory"
chmod 700 "$WORK"
QEMU_PID=
cleanup() {
    [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

HOME=$WORK/home
mkdir -p "$HOME"
export HOME
GIT_CONFIG_NOSYSTEM=1
GIT_TERMINAL_PROMPT=0
GIT_AUTHOR_NAME="Rivet proof"
GIT_AUTHOR_EMAIL="proof@example.invalid"
GIT_COMMITTER_NAME=$GIT_AUTHOR_NAME
GIT_COMMITTER_EMAIL=$GIT_AUTHOR_EMAIL
export GIT_CONFIG_NOSYSTEM GIT_TERMINAL_PROMPT GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL

# --- 1. the binaries --------------------------------------------------------

note "building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd -p rivetos-fleetd ) \
    || fail "the guest build failed"
note "building cargo-rivet and rivet-provision for the host"
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision ) || fail "the host tools did not build"
CARGO_RIVET=$HOST_OUT/cargo-rivet
PROVISION=$HOST_OUT/rivet-provision
( cd "$HERE/fixture-signer" && cargo build --release ) || fail "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer
[ -x "$SIGNER" ] || fail "no fixture signer at $SIGNER"

STRIP=${RIVET_STRIP:-$BR/host/bin/aarch64-linux-strip}
[ -x "$STRIP" ] || fail "no cross strip at $STRIP"
"$STRIP" -o "$WORK/$FLEETD" "$OUT/$FLEETD" || fail "rivet-fleetd could not be stripped"

# --- 2. throwaway identities and a throwaway fleet authority -----------------

mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$2" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; fail "could not mint the $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
note "minting THROWAWAY identities and an authority in $WORK"
GUEST_PIN=$(mint guest "$GUEST")
OPERATOR_PIN=$(mint operator rivet-adr044c-operator)
FLEET_OPERATOR_PIN=$(mint fleet-operator rivet-adr044c-fleet-operator)
MEMBER_PIN=$(mint member "$MEMBER")
for _pin in "$GUEST_PIN" "$OPERATOR_PIN" "$FLEET_OPERATOR_PIN" "$MEMBER_PIN"; do
    [ -n "$_pin" ] || fail "an identity has no readable public key"
done
mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/"
printf '# the operator, and this fleet server'"'"'s own operator identity\n%s\n%s\n' \
    "$OPERATOR_PIN" "$FLEET_OPERATOR_PIN" >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

HOME="$WORK/home" "$PROVISION" ca "$SUFFIX" code-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; fail "no authority"; }
for _host in "$GUEST" "$MEMBER"; do
    HOME="$WORK/home" "$PROVISION" ca server "$_host" "$WORK/ca" >>"$WORK/ca.log" 2>&1 </dev/null \
        || { cat "$WORK/ca.log" >&2; fail "no $_host pair"; }
done
SERVER_PAIR=$WORK/ca/server-$GUEST.$SUFFIX
MEMBER_PAIR=$WORK/ca/server-$MEMBER.$SUFFIX
[ -f "$SERVER_PAIR.crt" ] && [ -f "$MEMBER_PAIR.crt" ] && [ -f "$WORK/ca/browser.crt" ] \
    || fail "rivet-provision did not write every pair this proof presents"

# --- 3. rivet-fleetd's manifest, signature and staged material ---------------

{
    printf 'kind = "daemon"\nrestart = "on-failure"\nready = "none"\nnetwork = "egress"\n'
    printf 'state = "private"\nlisten = ["tcp:4443"]\nprovides = []\nconnects = []\n'
    printf 'library = []\ndevices = []\n'
    printf 'settings = ["fleet-repository", "fleet-branch", "fleet-poll-seconds"]\n'
} >"$WORK/$FLEETD.toml"
"$SIGNER" "$FLEETD" "$WORK/$FLEETD" "$WORK/$FLEETD.toml" "$WORK/$FLEETD.header" "$WORK/trust" >/dev/null \
    || fail "rivet-fleetd could not be signed"

hexof() { od -An -v -tx1 "$1" | tr -d ' \n'; }
mkdir -p "$WORK/register/devices"
printf 'name = "adr-044 fleet-code proof"\n' >"$WORK/register/register.toml"
{
    printf 'id = "%s"\nname = "%s"\nrole = "%s"\npin = "%s"\nlisten = "0.0.0.0:%s"\n' \
        "$DEVICE_ID" "$GUEST" "$SERVER_ROLE" "$GUEST_PIN" "$CONTROL_PORT"
    printf '\n[interfaces.eth0]\nmethod = "static"\naddress = "10.0.2.15/24"\ngateway = "10.0.2.2"\n'
} >"$WORK/register/devices/$DEVICE_ID.toml"
hexof "$WORK/fleet-operator/device.seed" >"$WORK/operator-device.seed.hex"
hexof "$WORK/fleet-operator/device.crt" >"$WORK/operator-device.crt.hex"

SEEDED_STATE=$WORK/state
mkdir -p "$SEEDED_STATE${STAGING#"$BUILT_IN"}"
BUNDLE=$SEEDED_STATE${STAGING#"$BUILT_IN"}/$FLEETD.txt
{
    printf '# a throwaway bundle, staged as a card stages one\nversion = 1\n'
    printf '%s server.crt %s\n' "$FLEETD" "$(hexof "$SERVER_PAIR.crt")"
    printf '%s server.key %s\n' "$FLEETD" "$(hexof "$SERVER_PAIR.key")"
    printf '%s ca.crt %s\n' "$FLEETD" "$(hexof "$WORK/ca/root.crt")"
    printf '%s operator-device.seed.hex %s\n' "$FLEETD" "$(hexof "$WORK/operator-device.seed.hex")"
    printf '%s operator-device.crt.hex %s\n' "$FLEETD" "$(hexof "$WORK/operator-device.crt.hex")"
    printf '%s register/register.toml %s\n' "$FLEETD" "$(hexof "$WORK/register/register.toml")"
    printf '%s register/devices/%s.toml %s\n' "$FLEETD" "$DEVICE_ID" "$(hexof "$WORK/register/devices/$DEVICE_ID.toml")"
} >"$BUNDLE"
chmod 700 "$(dirname "$BUNDLE")"
chmod 600 "$BUNDLE"

REPO_ID=$(openssl rand -hex 8)
DOC=$WORK/machine.toml
{
    printf 'version = 1\nhostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[services.%s]\nenabled = true\n' "$FLEETD"
    printf '\n[applications.%s]\nfleet-repository = "%s"\nfleet-poll-seconds = "3"\n' "$FLEETD" "$REPOSITORY"
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
    printf '\n[repos.%s]\nid = "%s"\n' "$REPOSITORY" "$REPO_ID"
} >"$DOC"

# --- 4. the machine ---------------------------------------------------------

CONSOLE=${RIVET_PROOF_LOG:-$REPO/out/adr-044-fleet-code-proof.console.log}
mkdir -p "$(dirname "$CONSOLE")"
: >"$CONSOLE"
mark() { wc -l <"$CONSOLE" | tr -d ' '; }
since() { tail -n "+$(($1 + 1))" "$CONSOLE"; }

note "booting: $FLEETD enabled and confined, [repos.$REPOSITORY] declared"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --git "$GUEST_GIT" \
    --git-http-backend "$GUEST_HTTP_BACKEND" \
    --secrets "$WORK/secrets" \
    --state "$SEEDED_STATE" \
    --document "$DOC" \
    --app "$FLEETD:$WORK/$FLEETD:$WORK/$FLEETD.toml:$WORK/$FLEETD.header" \
    --trust "$WORK/trust" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$FLEET_PORT:4443" \
    >"$CONSOLE" 2>&1 </dev/null &
QEMU_PID=$!

WAITED=0
while [ "$WAITED" -lt 240 ]; do
    grep -qF 'rivet-mcpd: listening on' "$CONSOLE" 2>/dev/null && break
    kill -0 "$QEMU_PID" 2>/dev/null || { tail -80 "$CONSOLE" >&2; fail "run.sh exited before the control plane listened"; }
    sleep 0.5
    WAITED=$((WAITED + 1))
done
grep -qF 'rivet-mcpd: listening on' "$CONSOLE" || { tail -60 "$CONSOLE" >&2; fail "no control plane within 120s"; }

ask() {
    set +e
    "$CARGO_RIVET" run --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/operator" --pin "$GUEST_PIN" \
        -- "$@" >"$WORK/ask-out" 2>"$WORK/ask-err"
    ASK_RC=$?
    set -e
}
ATTEMPT=1
while :; do
    ask get status
    [ "$ASK_RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 30 ] && { cat "$WORK/ask-err" >&2; fail "the machine never answered get status"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done
note "the machine answers get status (attempt $ATTEMPT)"

# `https PORT PATH PAIR [curl arguments]` — one request to rivet-fleetd presenting
# PAIR.crt/PAIR.key. CODE is the HTTP status, 000 when no response arrived.
https() {
    _path=$1 _pair=$2
    shift 2
    set +e
    CODE=$(curl -sS --max-time 60 --cert "$_pair.crt" --key "$_pair.key" --cacert "$WORK/ca/root.crt" \
        --resolve "$GUEST.$SUFFIX:$FLEET_PORT:127.0.0.1" -o "$WORK/https-out" -w '%{http_code}' "$@" \
        "https://$GUEST.$SUFFIX:$FLEET_PORT$_path" 2>"$WORK/https-err")
    set -e
}
BROWSER=$WORK/ca/browser

# ===========================================================================
STAGE="1 confined and serving"
# ===========================================================================
ATTEMPT=1
while :; do
    https / "$BROWSER"
    [ "$CODE" = 200 ] && break
    [ "$ATTEMPT" -ge 60 ] && { grep -a "$FLEETD" "$CONSOLE" | tail -30 >&2; fail "rivet-fleetd never served the operator: $CODE $(cat "$WORK/https-err")"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
grep -a 'ENFORCED this application is confined' "$CONSOLE" | grep -aq "$FLEETD" \
    || fail "no ENFORCED confinement record names $FLEETD, so nothing here is about a confined fleet server"
echo "--- the supervisor's confinement record for $FLEETD ---"
grep -a 'ENFORCED this application is confined' "$CONSOLE" | grep -a "$FLEETD" | head -1 | cut -c1-400
note "STAGE 1 passed: $FLEETD is confined by its Landlock ruleset and serves the operator on 4443"

# ===========================================================================
STAGE="2 the poller adopts the guest"
# ===========================================================================
openssl x509 -inform DER -in "$WORK/guest/device.crt" -out "$WORK/guest.crt.pem" || fail "the guest's certificate"
GUEST_SPKI=$(openssl x509 -in "$WORK/guest.crt.pem" -pubkey -noout \
    | openssl pkey -pubin -outform DER | openssl dgst -sha256 -binary | openssl base64 -A)
pem_key() {
    { printf '\060\056\002\001\000\060\005\006\003\053\145\160\004\042\004\040'; cat "$WORK/$1/device.seed"; } \
        | openssl pkey -inform DER -out "$WORK/$1.key.pem" 2>"$WORK/pem-err" \
        || { cat "$WORK/pem-err" >&2; fail "could not wrap $1's seed as a PEM key"; }
    openssl x509 -inform DER -in "$WORK/$1/device.crt" -out "$WORK/$1.crt.pem" || fail "could not read $1's certificate"
}
pem_key operator
pem_key member
URL="https://$GUEST:$CONTROL_PORT/$REPOSITORY.git"
stock_git() {
    git -c http.sslVerify=true -c "http.sslCAInfo=$WORK/guest.crt.pem" \
        -c "http.pinnedPubkey=sha256//$GUEST_SPKI" \
        -c "http.sslCert=$WORK/operator.crt.pem" -c "http.sslKey=$WORK/operator.key.pem" \
        -c "http.curloptResolve=$GUEST:$CONTROL_PORT:127.0.0.1" -c credential.helper= "$@"
}
remote_main() {
    stock_git ls-remote "$URL" refs/heads/main 2>"$WORK/ls-err" | cut -f1
}

ATTEMPT=1
while :; do
    ask get file "$BUILT_IN/git/$REPOSITORY.git/HEAD"
    [ "$ASK_RC" -eq 0 ] && grep -q 'refs/heads/main' "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 30 ] && fail "rivet-sysd never created $REPOSITORY.git"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done

CHECKOUT=$WORK/checkout
git init -q --initial-branch=main "$CHECKOUT"
mkdir -p "$CHECKOUT/$MACHINES_DIR"
cp "$REPO/examples/fleet/README.md" "$CHECKOUT/README.md"
# **Verbatim**: the guest's own document, so the first cycle adopts it and
# writes nothing.
cp "$DOC" "$CHECKOUT/$MACHINES_DIR/$GUEST.toml"
git -C "$CHECKOUT" add -A
git -C "$CHECKOUT" commit -q -m "The fleet server joins its own fleet, verbatim"
SEED=$(git -C "$CHECKOUT" rev-parse HEAD)
stock_git -C "$CHECKOUT" push -q "$URL" main 2>"$WORK/push-err" \
    || { cat "$WORK/push-err" >&2; fail "a stock git could not push the seed commit over the control plane"; }
[ "$(remote_main)" = "$SEED" ] || fail "main is not the seed commit after the push"

BASELINE=$DATA_DIR/$CODE_DIR/$BASELINES/$DEVICE_ID
ATTEMPT=1
while :; do
    ask get file "$BASELINE"
    [ "$ASK_RC" -eq 0 ] && grep -q "$SEED" "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 90 ] && { grep -a 'fleet-as-code' "$CONSOLE" | tail -30 >&2; fail "the poller never recorded $SEED as $GUEST's baseline"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
echo "--- get file $BASELINE, off the machine ---"
cat "$WORK/ask-out"
# `rivet-mcpd` names a caller by the first eight hex digits of its pinned key.
FLEET_OPERATOR_SHORT=$(printf '%.8s' "$FLEET_OPERATOR_PIN")
for _line in "POST /fleet-as-code/head over .*pinned key $FLEET_OPERATOR_SHORT.*: commit $SEED" \
    "POST /fleet-as-code/machines over .*pinned key $FLEET_OPERATOR_SHORT.*: machine $GUEST" \
    "POST /fleet-as-code/file over .*pinned key $FLEET_OPERATOR_SHORT.*: present"; do
    grep -aq "rivet-mcpd: fleet-as-code: $_line" "$CONSOLE" \
        || { grep -a 'rivet-mcpd: fleet-as-code' "$CONSOLE" | tail -10 >&2; fail "rivet-mcpd recorded no \"$_line\" from the fleet server's own operator key"; }
done
echo "--- what rivet-mcpd recorded of the poller's first answered questions ---"
grep -a "rivet-mcpd: fleet-as-code: POST .*pinned key $FLEET_OPERATOR_SHORT.*: \(commit\|machine\|present\) " "$CONSOLE" | head -3
if grep -a "$FLEETD" "$CONSOLE" | grep -aqi 'cannot run\|/system/bin/git'; then
    grep -a "$FLEETD" "$CONSOLE" | grep -ai 'cannot run\|/system/bin/git' >&2
    fail "rivet-fleetd tried to run a program"
fi
note "STAGE 2 passed: the confined poller read head, machines and the document through rivet-mcpd and adopted $GUEST at $SEED"

# ===========================================================================
STAGE="3 a commit through /api/commit converges the machine"
# ===========================================================================
json_string() {
    awk 'BEGIN { ORS = "" } { gsub(/\\/, "\\\\"); gsub(/"/, "\\\""); gsub(/\t/, "\\t"); print $0 "\\n" }' "$1"
}
sed 's/^hostname = .*/hostname = "converged-by-a-commit"/' "$DOC" >"$WORK/renamed.toml"
grep -q '^hostname = "converged-by-a-commit"$' "$WORK/renamed.toml" || fail "the renamed document was not composed"
{
    printf '{"expected":"'; json_string "$DOC"
    printf '","document":"'; json_string "$WORK/renamed.toml"
    printf '","message":"Rename %s through the portal"}' "$GUEST"
} >"$WORK/commit.json"

MARK_COMMIT=$(mark)
https "/api/commit/$GUEST" "$BROWSER" -H 'Content-Type: application/json' --data-binary "@$WORK/commit.json"
echo "--- POST /api/commit/$GUEST as the operator: $CODE ---"
cat "$WORK/https-out"; echo
[ "$CODE" = 200 ] || fail "the commit was not answered 200: $(cat "$WORK/https-err")"
COMMITTED=$(grep -oE '"commit":"[0-9a-f]{40}"' "$WORK/https-out" | grep -oE '[0-9a-f]{40}')
[ -n "$COMMITTED" ] || fail "the answer names no commit"
[ "$(remote_main)" = "$COMMITTED" ] || fail "main is not $COMMITTED after the commit"
[ "$(git -C "$CHECKOUT" rev-parse "$SEED")" = "$SEED" ] || fail "the checkout lost the seed"
stock_git -C "$CHECKOUT" fetch -q "$URL" main 2>"$WORK/fetch-err" || { cat "$WORK/fetch-err" >&2; fail "could not fetch main back"; }
[ "$(git -C "$CHECKOUT" rev-parse "$COMMITTED^")" = "$SEED" ] || fail "$COMMITTED is not a child of the seed"
git -C "$CHECKOUT" show "$COMMITTED:$MACHINES_DIR/$GUEST.toml" | grep -q '^hostname = "converged-by-a-commit"$' \
    || fail "the committed document does not carry the new hostname"
note "the commit $COMMITTED is main, a child of $SEED, made by $(git -C "$CHECKOUT" log -1 --format='%an' "$COMMITTED")"

SHORT=$(printf '%.12s' "$COMMITTED")
ATTEMPT=1
while :; do
    ask get desired-state
    [ "$ASK_RC" -eq 0 ] && grep -q 'converged-by-a-commit' "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 90 ] && { since "$MARK_COMMIT" | grep -a 'fleet-as-code' | tail -30 >&2; fail "the machine never carried the committed hostname"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
since "$MARK_COMMIT" | grep -aq "is converged onto $SHORT" \
    || { since "$MARK_COMMIT" | grep -a 'fleet-as-code' | tail -20 >&2; fail "the poller did not say it converged $GUEST onto $SHORT"; }
echo "--- the poller, verbatim ---"
since "$MARK_COMMIT" | grep -a "is converged onto $SHORT" | head -1
ATTEMPT=1
while :; do
    ask get file "$BASELINE"
    [ "$ASK_RC" -eq 0 ] && grep -q "$COMMITTED" "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 15 ] && fail "the baseline does not name $COMMITTED"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
note "STAGE 3 passed: POST /api/commit committed $COMMITTED, the poller saw the new head, wrote $GUEST, and get desired-state says converged-by-a-commit"

# ===========================================================================
STAGE="4 a stale compare-and-swap is refused"
# ===========================================================================
https "/api/commit/$GUEST" "$BROWSER" -H 'Content-Type: application/json' --data-binary "@$WORK/commit.json"
echo "--- the same POST again, its expected document now stale: $CODE ---"
cat "$WORK/https-out"; echo
grep -q '"code":-32010' "$WORK/https-out" && grep -q 'REFUSED' "$WORK/https-out" \
    && grep -q 'has changed since it was read' "$WORK/https-out" \
    || fail "a stale expected document was not refused by rivet-fleetd"
[ "$(remote_main)" = "$COMMITTED" ] || fail "main moved during a refused commit"

DOC_BYTES=$(wc -c <"$WORK/renamed.toml" | tr -d ' ')
printf 'A commit composed against the seed\n' >"$WORK/stale-message"
MSG_BYTES=$(wc -c <"$WORK/stale-message" | tr -d ' ')
{
    printf 'repository %s\nbranch main\nexpected-head %s\npath %s/%s.toml\n' "$REPOSITORY" "$SEED" "$MACHINES_DIR" "$GUEST"
    printf 'message-bytes %s\ndocument-bytes %s\n\n' "$MSG_BYTES" "$DOC_BYTES"
    cat "$WORK/stale-message" "$WORK/renamed.toml"
} >"$WORK/stale-commit"
control() {
    _who=$1 _route=$2 _body=$3
    set +e
    CODE=$(curl -sS --max-time 60 --cacert "$WORK/guest.crt.pem" --pinnedpubkey "sha256//$GUEST_SPKI" \
        --cert "$WORK/$_who.crt.pem" --key "$WORK/$_who.key.pem" \
        --resolve "$GUEST:$CONTROL_PORT:127.0.0.1" -H 'Content-Type: text/plain' \
        --data-binary "@$_body" -o "$WORK/control-out" -w '%{http_code}' \
        "https://$GUEST:$CONTROL_PORT$_route" 2>"$WORK/control-err")
    set -e
}
control operator /fleet-as-code/commit "$WORK/stale-commit"
echo "--- POST /fleet-as-code/commit on 7777, expected-head the seed: $CODE ---"
cat "$WORK/control-out"
[ "$CODE" = 409 ] && grep -q '^refused stale-head$' "$WORK/control-out" \
    || fail "a stale expected-head was not refused 409 stale-head: $CODE $(cat "$WORK/control-err")"
[ "$(remote_main)" = "$COMMITTED" ] || fail "main moved during a stale-head refusal"
note "STAGE 4 passed: a stale document refused by rivet-fleetd, a stale head refused 409 by rivet-mcpd, main still $COMMITTED"

# ===========================================================================
STAGE="5 a fleet member is refused"
# ===========================================================================
https "/api/commit/$GUEST" "$MEMBER_PAIR" -H 'Content-Type: application/json' --data-binary "@$WORK/commit.json"
echo "--- POST /api/commit/$GUEST with $MEMBER's front-door certificate: $CODE ---"
cat "$WORK/https-out"; echo
[ "$CODE" = 403 ] && grep -q '"code":-32003' "$WORK/https-out" \
    || fail "$MEMBER's front-door certificate was not refused at 4443: $CODE"

printf 'repository %s\nbranch main\n' "$REPOSITORY" >"$WORK/head-request"
control operator /fleet-as-code/head "$WORK/head-request"
[ "$CODE" = 200 ] && grep -q "^commit $COMMITTED\$" "$WORK/control-out" \
    || fail "the operator's control-plane identity could not read the head: $CODE $(cat "$WORK/control-out" "$WORK/control-err")"
control member /fleet-as-code/head "$WORK/head-request"
echo "--- POST /fleet-as-code/head with $MEMBER's control-plane identity, not in peers: $CODE ---"
cat "$WORK/control-err"
[ "$CODE" = 000 ] || fail "$MEMBER's control-plane identity was answered $CODE on 7777"
control member /fleet-as-code/commit "$WORK/stale-commit"
[ "$CODE" = 000 ] || fail "$MEMBER's control-plane identity was answered $CODE for a commit"
[ "$(remote_main)" = "$COMMITTED" ] || fail "main moved after refused members"
note "STAGE 5 passed: $MEMBER refused at 4443 (403) and on 7777 (no request completed); main still $COMMITTED"

echo
echo "adr-044-fleet-code-proof: PASS"
echo
echo "  What ran, on $RIVET_KERNEL under QEMU -M virt:"
echo "    * the real rivet-fleetd, confined by its own Landlock ruleset, polling"
echo "      [repos.$REPOSITORY] through rivet-mcpd with no git of its own;"
echo "    * a verbatim document pushed by a stock git was adopted ($SEED);"
echo "    * POST /api/commit committed $COMMITTED, the poller converged the guest"
echo "      onto it, and get desired-state carried the new hostname;"
echo "    * a stale document and a stale head were refused and main did not move;"
echo "    * a member's front-door certificate and control-plane identity were refused."
echo
echo "  What it does NOT settle: a board, a card's ext4 /state, rivet-netd's own"
echo "  network path, or any machine but the fleet server itself. The console is"
echo "  $CONSOLE."
