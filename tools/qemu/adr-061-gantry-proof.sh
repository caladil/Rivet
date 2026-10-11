#!/bin/sh
# tools/qemu/adr-061-gantry-proof.sh — **Gantry's half of a fleet server, on a
# booted guest: the `gantry` socket from a real client under the real uid, an
# APK published with `--sync confirm` into the guest's store, the phone verbs,
# and the proxy's port 80.**
#
# `ADR-061` §6, §7 and §9, `ADR-052` §7, `SPEC-06`. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-061-gantry-proof.sh
#
# ## What is on the guest
#
# BOOT 1 is its own fleet server: the control plane (`rivet-mcpd`,
# `rivet-netd`), the installer (`rivet-deploy`), the vendored `git` and
# `git-http-backend` with `[repos.fleet]` declared, the image's `rivet-proxyd`
# (`--proxyd`) holding 443 and — because an installed application declares a
# `plain:` rule — 80, and three applications installed with `--app`, signed by
# the fixture key and ENABLED:
#
# * the real `rivet-fleetd`, stripped, with its certificates and a one-record
#   register naming the guest `role = "fleet-server"` staged under
#   `/state/staging` as a card stages them;
# * `tools/qemu/gantry-probe` as `gantry`, declaring
#   `connects = ["rivet-fleetd:gantry"]`, `tcp:8080` behind
#   `proxy-path = "gantry"` and `proxy-auth-paths = ["plain:install", "app:api"]`;
# * the same binary as `stranger`, declaring nothing, which dials the
#   provider's socket path directly.
#
# The trust store holds three keys, each minted or fixed by this run: the
# fixture key (`application`, for the applications), a throwaway application
# key with `apk` in its mask, and a second throwaway key WITHOUT `apk`.
#
# BOOT 2 carries the proxy and a `gantry` whose manifest declares no `plain:`
# rule, and nothing else of note.
#
# BOOT 3 carries `rivet-fleetd`, `gantry` and the same binary as `not-gantry`,
# declaring the same `connects` as gantry. The gantry socket is not shared, so
# two applications claiming it is a contest, and a contest is refused to every
# claimant (`ADR-015` §5.3) — gantry included.
#
# ## What it asserts — one OK or FAIL line each
#
# 1. **The socket.** fleetd journals serving the gantry socket to exactly the
#    uid gantry's data directory is owned by; the probe's `fleet`,
#    `artifacts`, `enrol` of a HAND-COMPOSED P-256 key, and `phones` are each
#    answered, the identity is the SHA-256 of the key the reply carries, and
#    `phones` lists it. `stranger` is not answered, and the proof prints which
#    lock stopped it.
# 2. **`--sync confirm` with a real APK** (`cmd/cargo-rivet/tests/fixtures/apk/
#    rivet-harness-derived.apk`, real APK structure signed with throwaway
#    keys): the object is held in fleetd's store, read back with `get file`,
#    and the probe's next `artifacts` lists it. An `apk` signed by the key
#    without `apk` is refused at the door and not held. `cargo rivet install`
#    of the held apk artifact is refused by the guest's installer, by name.
# 2b. **Hide, unhide and retire** (`ADR-052` §7). The same APK is published
#    again under a second product name, so the store holds two products. `fleet
#    hide` of the first commits a `[[hidden]]` entry; after `fleet push …
#    confirm` and `fleet sync … --prune confirm` the sync report names its
#    object `hidden` and does not send it, the prune removes exactly that
#    object, the second product's survives, and the probe's next `artifacts`
#    no longer offers the hidden one. `fleet unhide`, push and sync send it
#    again, the store holds it and `artifacts` offers it. `fleet retire` of the
#    second product, push and `--prune confirm` remove exactly its object and
#    `artifacts` stops offering it.
# 3. **The phone verbs.** `fleet phones list` shows the enrolled phone;
#    `rename … confirm` changes its label and the probe's next `phones` shows
#    the new label (so fleetd can read the file the control plane rewrote);
#    `revoke … confirm` removes it and the probe's next `phones` is empty.
# 4. **Port 80.** The supervisor journals rivet-proxyd bound on 443 and 80;
#    over plain HTTP the plain route's `GET` is 200, a `POST` is 405, a
#    non-plain path is 404 and `/` is 404; on 443 the `app:` route is forwarded
#    with no client certificate and the probe echoes the `Authorization`
#    header unchanged. BOOT 2, with no plain rule: rivet-proxyd is bound on
#    443 alone and the forwarded port 80 answers nothing.
# 5. **Two claimants.** BOOT 3: the supervisor refuses both `gantry` and
#    `not-gantry`, each refusal naming both, and neither runs a round.
#
# ## What it does NOT settle
#
# `ADR-026` §4: no Raspberry Pi, no SD card, no phone. `/state` is the
# initramfs tmpfs. The probe is a fixture speaking `SPEC-06` and serving two
# routes, not Gantry; the phone key is hand-composed and no phone proved
# possession of it. SLIRP's forward stands in for a LAN. Every key, identity and
# seed is THROWAWAY, minted into this run's working directory.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release
HOST_OUT=$TARGET_DIR/release

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER
BR=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}
CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$BR/host/bin/aarch64-linux-gcc}"
export CC_aarch64_unknown_linux_musl

NAME=adr-061-gantry-proof
VERDICT=0
ok() { echo "  OK   $1"; }
fail() {
    echo "  FAIL $1"
    VERDICT=1
}
die() {
    echo "$NAME: $1" >&2
    exit 1
}
note() { echo "$NAME: $*"; }
# A file's lines, indented, each ending in a newline whether or not the file's
# last one did.
show() { awk '{ print "    | " $0 }' "$1"; }

command -v qemu-system-aarch64 >/dev/null 2>&1 || die "qemu-system-aarch64 is not on PATH"
for _tool in git openssl curl od base64; do
    command -v "$_tool" >/dev/null 2>&1 || die "no $_tool on PATH"
done
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at $CC_aarch64_unknown_linux_musl"
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi
[ -n "${RIVET_KERNEL:-}" ] || die "no RIVET_KERNEL and no /br/images/Image"
GUEST_GIT=$BR/target/usr/bin/git
GUEST_HTTP_BACKEND=$BR/target/usr/libexec/git-core/git-http-backend
[ -f "$GUEST_GIT" ] || die "no vendored git at $GUEST_GIT"
[ -f "$GUEST_HTTP_BACKEND" ] || die "no git-http-backend at $GUEST_HTTP_BACKEND"
APK=$REPO/cmd/cargo-rivet/tests/fixtures/apk/rivet-harness-derived.apk
[ -f "$APK" ] || die "no APK fixture at $APK"

# --- constants read out of the crates that own them -------------------------

constant() {
    _value=$(sed -n "s/^pub const $2: *&str *= *\"\([^\"]*\)\".*/\1/p" "$1" | head -1)
    [ -n "$_value" ] || die "cannot read $2 out of $1 — this harness agrees with that crate and will not guess"
    printf '%s\n' "$_value"
}
CONTROL_PORT=$(sed -n 's/^pub const DEFAULT_PORT: *u16 *= *\([0-9]*\);.*/\1/p' "$REPO/rivet-config/src/control_plane.rs")
[ -n "$CONTROL_PORT" ] || die "cannot read DEFAULT_PORT"
MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
STAGING=$(constant "$REPO/rivet-core/src/state.rs" STAGING)
BUILT_IN=$(constant "$REPO/rivet-core/src/service.rs" BUILT_IN_VOLUME)
SERVER_ROLE=$(constant "$REPO/rivet-core/src/fleet_as_code.rs" SERVER_ROLE)
FLEETD=$(constant "$REPO/rivet-core/src/repository_objects.rs" APPLICATION)
STORE_DIR=$(constant "$REPO/rivet-core/src/repository_objects.rs" DIR)
REGISTER_DIR=$(constant "$REPO/cmd/rivet-fleetd/src/fleet.rs" REGISTER_DIR)
SERVED=$(constant "$REPO/rivet-fleet-repo/src/lib.rs" SERVED_NAME)
APPS=$BUILT_IN/app
FLEETD_DATA=$APPS/$FLEETD/data
OBJECTS=$FLEETD_DATA/$STORE_DIR/objects
PHONES=$FLEETD_DATA/$REGISTER_DIR/phones
PROXY_PORT=8443
PLAIN_PORT=18080
SUFFIX=local
GUEST=gantry-guest
MACHINE=$GUEST.$SUFFIX
DEVICE_ID=0a1b2c3d4e5f6071
FLEET_NAME="adr-061 gantry proof"

WORK=${TMPDIR:-/tmp}/$NAME
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"
QEMU_PID=
cleanup() {
    [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null
    return 0
}
trap cleanup EXIT

# **Nothing here may read this host's git configuration, credentials, register
# or ~/.rivet.**
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
unset RIVET_APP_SEED RIVET_SYSTEM_SEED RIVET_FLEET_REPOSITORY RIVET_IDENTITY RIVET_AT 2>/dev/null || true
RIVET_FLEET_REGISTER=$WORK/register
export RIVET_FLEET_REGISTER

# **The vendored git may be dynamic** (run.sh's --git says why), so the
# libraries it NEEDs are staged beside the loader run.sh's --libraries packs,
# found by the cross readelf in the Buildroot tree that built them.
READELF=$BR/host/bin/aarch64-linux-readelf
[ -x "$READELF" ] || die "no cross readelf at $READELF"
mkdir -p "$WORK/libs"
stage_needed() {
    for _lib in $("$READELF" -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p'); do
        [ "$_lib" = libc.so ] && continue
        [ -e "$WORK/libs/$_lib" ] && continue
        _found=
        for _dir in "$BR/target/usr/lib" "$BR/target/lib"; do
            [ -e "$_dir/$_lib" ] && { _found=$_dir/$_lib; break; }
        done
        [ -n "$_found" ] || die "$1 NEEDs $_lib and no library directory under $BR/target holds it"
        cp -L "$_found" "$WORK/libs/$_lib"
        stage_needed "$WORK/libs/$_lib"
    done
}
stage_needed "$GUEST_GIT"
stage_needed "$GUEST_HTTP_BACKEND"

# --- 1. the binaries --------------------------------------------------------

note "building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd \
    -p rivetos-deploy -p rivetos-proxyd -p rivetos-fleetd ) || die "the guest build failed"
( cd "$HERE/gantry-probe" && cargo build --release --target "$TARGET" ) \
    || die "tools/qemu/gantry-probe did not build"
[ -f "$OUT/gantry-probe" ] || die "the probe was not built at $OUT/gantry-probe"
note "building cargo-rivet, rivet-provision and the fixture signer for the host"
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision ) || die "the host tools did not build"
CARGO_RIVET=$HOST_OUT/cargo-rivet
PROVISION=$HOST_OUT/rivet-provision
( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer
[ -x "$SIGNER" ] || die "no fixture signer at $SIGNER"

STRIP=${RIVET_STRIP:-$BR/host/bin/aarch64-linux-strip}
[ -x "$STRIP" ] || die "no cross strip at $STRIP"
"$STRIP" -o "$WORK/$FLEETD" "$OUT/$FLEETD" || die "rivet-fleetd could not be stripped"
"$STRIP" -o "$WORK/rivet-proxyd" "$OUT/rivet-proxyd" || die "rivet-proxyd could not be stripped"

# --- 2. keys: the fixture's public half, and two THROWAWAY seeds ------------
#
# The fixture signer's seed is thirty-two bytes of 0x2a and is not a key
# (`tools/qemu/fixture-signer`); its public half is asked of `cargo rivet key`
# so the trust store can name it beside the two throwaway keys.
pub_of() {
    "$CARGO_RIVET" key "$1" --out "$2" >"$WORK/key.log" 2>&1 || { cat "$WORK/key.log" >&2; die "no public key for $1"; }
}
printf '2a%.0s' 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 \
    >"$WORK/fixture.seed"
pub_of "$WORK/fixture.seed" "$WORK/fixture.pub"
openssl rand -hex 32 >"$WORK/apk.seed" || die "could not mint a throwaway seed"
pub_of "$WORK/apk.seed" "$WORK/apk.pub"
openssl rand -hex 32 >"$WORK/no-apk.seed" || die "could not mint a throwaway seed"
pub_of "$WORK/no-apk.seed" "$WORK/no-apk.pub"
TRUST=$WORK/trust
"$SIGNER" trust-store "$TRUST" "qemu-fixture:application:$WORK/fixture.pub" \
    "throwaway-apk:application,apk:$WORK/apk.pub" "throwaway-no-apk:application:$WORK/no-apk.pub" \
    >"$WORK/trust.log" 2>&1 || { cat "$WORK/trust.log" >&2; die "could not write the trust store"; }
cat "$WORK/trust.log"

# --- 3. identities, peers and a throwaway fleet authority -------------------

mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$2" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; die "could not mint the $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
note "minting THROWAWAY identities and an authority in $WORK"
GUEST_PIN=$(mint guest "$GUEST")
OPERATOR_PIN=$(mint operator adr061-operator)
FLEET_OPERATOR_PIN=$(mint fleet-operator adr061-fleet-operator)
for _pin in "$GUEST_PIN" "$OPERATOR_PIN" "$FLEET_OPERATOR_PIN"; do
    [ -n "$_pin" ] || die "an identity has no readable public key"
done
mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/"
printf '# the operator, and the fleet server'"'"'s own identity\n%s\n%s\n' \
    "$OPERATOR_PIN" "$FLEET_OPERATOR_PIN" >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

"$PROVISION" ca "$SUFFIX" gantry-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no authority"; }
"$PROVISION" ca server "$GUEST" "$WORK/ca" >>"$WORK/ca.log" 2>&1 </dev/null \
    || { cat "$WORK/ca.log" >&2; die "no server pair for $GUEST"; }
SERVER_PAIR=$WORK/ca/server-$MACHINE
[ -f "$SERVER_PAIR.crt" ] && [ -f "$WORK/ca/root.crt" ] || die "rivet-provision did not write the pairs"

# --- 4. manifests, rendered by the tool that renders them for a package -----

render_manifest() {
    _name=$1
    _meta=$2
    mkdir -p "$WORK/manifests/$_name"
    {
        printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2024"\n\n' "$_name"
        printf '[package.metadata.rivet]\n%s' "$_meta"
    } >"$WORK/manifests/$_name/Cargo.toml"
    "$CARGO_RIVET" package --manifest-path "$WORK/manifests/$_name/Cargo.toml" --print-manifest \
        >"$WORK/$_name.toml" 2>"$WORK/render.err" || { cat "$WORK/render.err" >&2; die "no manifest for $_name"; }
}
DAEMON='kind = "daemon"
restart = "on-failure"
ready = "none"
network = "none"
state = "private"
'
GANTRY_META="${DAEMON}listen = [\"tcp:8080\"]
connects = [\"$FLEETD:gantry\"]
proxy-path = \"gantry\"
proxy-auth-paths = [\"plain:install\", \"app:api\"]
"
render_manifest gantry "$GANTRY_META"
render_manifest not-gantry "${DAEMON}connects = [\"$FLEETD:gantry\"]
"
render_manifest stranger "$DAEMON"
"$CARGO_RIVET" package --manifest-path "$REPO/cmd/rivet-fleetd/Cargo.toml" --print-manifest \
    >"$WORK/$FLEETD.toml" 2>"$WORK/render.err" || { cat "$WORK/render.err" >&2; die "no manifest for $FLEETD"; }
grep -q '^provides = \["socket:gantry"\]$' "$WORK/$FLEETD.toml" || die "rivet-fleetd's manifest provides no gantry socket"
echo "--- the manifests, as rendered ---"
for _name in "$FLEETD" gantry not-gantry stranger; do
    echo "# $_name"
    cat "$WORK/$_name.toml"
done

sign() {
    "$SIGNER" "$1" "$2" "$WORK/$1.toml" "$WORK/$1.header" "$WORK/unused-trust" >/dev/null \
        || die "$1 could not be signed"
}
sign "$FLEETD" "$WORK/$FLEETD"
for _name in gantry not-gantry stranger; do
    cp "$OUT/gantry-probe" "$WORK/$_name"
    sign "$_name" "$WORK/$_name"
done

# --- 5. rivet-fleetd's staged material, and the workstation's register ------

hexof() { od -An -v -tx1 "$1" | tr -d ' \n'; }
mkdir -p "$WORK/register/devices"
printf 'name = "%s"\n' "$FLEET_NAME" >"$WORK/register/register.toml"
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
    printf '%s %s/register.toml %s\n' "$FLEETD" "$REGISTER_DIR" "$(hexof "$WORK/register/register.toml")"
    printf '%s %s/devices/%s.toml %s\n' "$FLEETD" "$REGISTER_DIR" "$DEVICE_ID" \
        "$(hexof "$WORK/register/devices/$DEVICE_ID.toml")"
} >"$BUNDLE"
chmod 700 "$(dirname "$BUNDLE")"
chmod 600 "$BUNDLE"

REPO_ID=$(openssl rand -hex 8)
DOC=$WORK/machine.toml
{
    printf 'version = 1\nhostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for _app in "$FLEETD" rivet-proxyd gantry stranger; do
        printf '\n[services.%s]\nenabled = true\n' "$_app"
    done
    printf '\n[applications.rivet-proxyd]\nupstream-name = "%s"\n' "$MACHINE"
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
    printf '\n[repos.%s]\nid = "%s"\n' "$SERVED" "$REPO_ID"
} >"$DOC"

# --- 6. BOOT 1 --------------------------------------------------------------

CONSOLE=$WORK/console-1.log
boot() {
    _console=$1
    shift
    "$HERE/run.sh" "$OUT/rivet-initd" \
        --supervisor "$OUT/rivet-sysd" \
        --cli "$OUT/rivet" \
        --netd "$OUT/rivet-netd" \
        --mcpd "$OUT/rivet-mcpd" \
        --secrets "$WORK/secrets" \
        --trust "$TRUST" \
        --proxyd "$WORK/rivet-proxyd" \
        --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$PROXY_PORT:443" \
        --forward-port "$PLAIN_PORT:80" \
        "$@" >"$_console" 2>&1 </dev/null &
    QEMU_PID=$!
    _waited=0
    while [ "$_waited" -lt 240 ]; do
        grep -qF 'rivet-mcpd: listening on' "$_console" 2>/dev/null && break
        kill -0 "$QEMU_PID" 2>/dev/null || { tail -80 "$_console" >&2; die "run.sh exited before the control plane listened"; }
        sleep 0.5
        _waited=$((_waited + 1))
    done
    grep -qF 'rivet-mcpd: listening on' "$_console" || { tail -60 "$_console" >&2; die "no control plane within 120s"; }
    _attempt=1
    while :; do
        ask get status
        [ "$ASK_RC" -eq 0 ] && break
        [ "$_attempt" -ge 30 ] && { cat "$WORK/ask-err" >&2; die "the machine never answered get status"; }
        _attempt=$((_attempt + 1))
        sleep 1
    done
}
ENDPOINT_ARGS="--host 127.0.0.1:$CONTROL_PORT --identity $WORK/operator --pin"
ask() {
    set +e
    # shellcheck disable=SC2086
    "$CARGO_RIVET" run $ENDPOINT_ARGS "$GUEST_PIN" -- "$@" >"$WORK/ask-out" 2>"$WORK/ask-err"
    ASK_RC=$?
    set -e
}
# `cargo rivet VERB … --host … --identity … --pin …`; output in $WORK/cr-out.
cr() {
    _verb=$1
    shift
    set +e
    # shellcheck disable=SC2086
    "$CARGO_RIVET" $_verb "$@" >"$WORK/cr-out" 2>&1
    CR_RC=$?
    set -e
}
# A `get file` answer with its JSON string escapes for quotes undone, so a
# reply line the probe wrote reads as the provider wrote it.
unescaped() { sed -e 's/\\"/"/g' "$WORK/ask-out"; }
# `read_file PATH` — `get file PATH`'s whole answer, unescaped, in
# $WORK/content. Empty when the machine did not answer.
read_file() {
    ask get file "$1"
    if [ "$ASK_RC" -eq 0 ]; then unescaped >"$WORK/content"; else : >"$WORK/content"; fi
}
# The number a probe last wrote to `done`, or 0.
done_of() {
    read_file "$APPS/$1/data/done"
    DONE=$(grep -oE '"content": ?"[0-9]+' "$WORK/content" | grep -oE '[0-9]+$' | head -1 || true)
    DONE=${DONE:-0}
}
# `wait_round APP N` — until APP has finished a round numbered above N.
wait_round() {
    _attempt=1
    while :; do
        done_of "$1"
        [ "$DONE" -gt "$2" ] && return 0
        [ "$_attempt" -ge 60 ] && return 1
        _attempt=$((_attempt + 1))
        sleep 2
    done
}
restart_gantry() {
    done_of gantry
    _before=$DONE
    ask apply service-restart gantry confirm
    [ "$ASK_RC" -eq 0 ] || { cat "$WORK/ask-out" "$WORK/ask-err" >&2; die "service-restart gantry exited $ASK_RC"; }
    wait_round gantry "$_before" || die "gantry never finished a round after its restart"
}
journal() {
    ask get log latest all
    cat "$WORK/ask-out" >"$WORK/journal"
}
held_digests() {
    ask get file "$OBJECTS"
    if [ "$ASK_RC" -ne 0 ]; then : >"$WORK/held"; return 0; fi
    grep -oE '[0-9a-f]{64}' "$WORK/ask-out" | sort -u >"$WORK/held"
}

note "BOOT 1: $FLEETD, gantry and stranger enabled; the proxy with a plain rule"
boot "$CONSOLE" \
    --deploy "$OUT/rivet-deploy" \
    --git "$GUEST_GIT" --git-http-backend "$GUEST_HTTP_BACKEND" --libraries "$WORK/libs" \
    --state "$SEEDED_STATE" \
    --document "$DOC" \
    --app "$FLEETD:$WORK/$FLEETD:$WORK/$FLEETD.toml:$WORK/$FLEETD.header" \
    --app "gantry:$WORK/gantry:$WORK/gantry.toml:$WORK/gantry.header" \
    --app "stranger:$WORK/stranger:$WORK/stranger.toml:$WORK/stranger.header"
note "the machine answers get status"

# ===========================================================================
echo
echo "--- 1. the gantry socket, from a real client under the real uid ---"
# ===========================================================================
wait_round gantry 0 || { journal; grep -a 'gantry' "$WORK/journal" | tail -30 >&2; die "gantry never finished its first round"; }
wait_round stranger 0 || die "stranger never finished its first round"

read_file "$APPS/gantry/data"
GANTRY_UID=$(grep -oE '"uid": ?[0-9]+' "$WORK/content" | grep -oE '[0-9]+$' | head -1 || true)
read_file "$FLEETD_DATA"
FLEETD_UID=$(grep -oE '"uid": ?[0-9]+' "$WORK/content" | grep -oE '[0-9]+$' | head -1 || true)
journal
SERVING=$(grep -ahoE 'gantry socket: serving uid [0-9]+ only' "$WORK/journal" "$CONSOLE" | head -1 || true)
if [ -n "$GANTRY_UID" ] && [ "$SERVING" = "gantry socket: serving uid $GANTRY_UID only" ] \
    && [ "$GANTRY_UID" != "$FLEETD_UID" ]; then
    ok "$FLEETD journals \"$SERVING\"; $APPS/gantry/data is owned by uid $GANTRY_UID"
else
    fail "$FLEETD's serving record '$SERVING' does not name gantry's uid '$GANTRY_UID' (fleetd is '$FLEETD_UID')"
fi

read_file "$APPS/gantry/data/last-round.txt"
echo "    gantry's round: $(grep -oE '"content": ?"[^"]*' "$WORK/content" | head -1 | cut -c1-200)"

read_file "$APPS/gantry/data/last-fleet.json"
if grep -qF "{\\\"ok\\\":true,\\\"name\\\":\\\"$FLEET_NAME\\\"}" "$WORK/ask-out" \
    || grep -qF "{\"ok\":true,\"name\":\"$FLEET_NAME\"}" "$WORK/content"; then
    ok "fleet is answered with the register's name, \"$FLEET_NAME\""
else
    fail "fleet was not answered with the register's name: $(cat "$WORK/content")"
fi

read_file "$APPS/gantry/data/last-artifacts.json"
if grep -qF '{"ok":true,"artifacts":[]}' "$WORK/content"; then
    ok "artifacts is answered, and an empty store is an empty array"
else
    fail "artifacts was not answered with an empty array: $(cat "$WORK/content")"
fi

read_file "$APPS/gantry/data/last-enrol.json"
cp "$WORK/content" "$WORK/enrol"
IDENTITY=$(grep -oE '"phone":\{"identity":"[A-Za-z0-9_-]+"' "$WORK/enrol" | grep -oE '[A-Za-z0-9_-]+"$' | tr -d '"' || true)
PUBLIC_KEY=$(grep -oE '"public_key":"[A-Za-z0-9+/=]+"' "$WORK/enrol" | cut -d'"' -f4 || true)
DERIVED=$(printf '%s' "$PUBLIC_KEY" | base64 -d 2>/dev/null | openssl dgst -sha256 -binary | base64 | tr '+/' '-_' | tr -d '=\n')
KEY_BYTES=$(printf '%s' "$PUBLIC_KEY" | base64 -d 2>/dev/null | wc -c | tr -d ' ')
if grep -qF '{"ok":true,"phone":{' "$WORK/enrol" && [ -n "$IDENTITY" ] && [ "$IDENTITY" = "$DERIVED" ] \
    && [ "$KEY_BYTES" = 91 ] && grep -qF '"label":"Hand-composed probe phone"' "$WORK/enrol"; then
    ok "enrol of the HAND-COMPOSED 91-byte P-256 key is answered; identity $IDENTITY is the SHA-256 of that key"
else
    fail "enrol was not answered as SPEC-06 says (identity '$IDENTITY', derived '$DERIVED', $KEY_BYTES key bytes): $(cat "$WORK/enrol")"
fi

read_file "$APPS/gantry/data/last-phones.json"
if [ -n "$IDENTITY" ] && grep -qF "{\"ok\":true,\"phones\":[{\"identity\":\"$IDENTITY\",\"label\":\"Hand-composed probe phone\"" "$WORK/content"; then
    ok "phones lists the enrolled phone"
else
    fail "phones does not list $IDENTITY: $(cat "$WORK/content")"
fi

read_file "$APPS/stranger/data/last-round.txt"
STRANGER_ROUND=$(grep -oE '"content": ?"[^"]*' "$WORK/content" | head -1 \
    | sed -e 's/^"content": \{0,1\}"//' -e 's/\\n$//')
read_file "$APPS/stranger/data/last-fleet.json"
if printf '%s' "$STRANGER_ROUND" | grep -q 'connect: refused by the kernel'; then
    ok "stranger is not answered: the KERNEL refused its connect to the 0600 socket ($(printf '%s' "$STRANGER_ROUND" | sed 's/\\n/; /g' | cut -c1-240))"
elif grep -qF '{"ok":false,"error":"refused"' "$WORK/content"; then
    ok "stranger is not answered: $FLEETD refused its uid ($(printf '%s' "$STRANGER_ROUND" | sed 's/\\n/; /g' | cut -c1-240))"
else
    fail "stranger was answered, or recorded nothing: round '$STRANGER_ROUND', fleet '$(cat "$WORK/content")'"
fi

# ===========================================================================
echo
echo "--- 2. fleet publish app.apk --sync confirm, into the guest's store ---"
# ===========================================================================
FLEET=$WORK/fleet
cr fleet init --at "$FLEET"
[ "$CR_RC" -eq 0 ] || { cat "$WORK/cr-out" >&2; die "fleet init"; }
cp "$APK" "$WORK/rivet-harness.apk"
MARK=$(wc -l <"$CONSOLE" | tr -d ' ')
# shellcheck disable=SC2086
cr fleet publish "$WORK/rivet-harness.apk" --signing-key "$WORK/apk.seed" --at "$FLEET" \
    --sync $ENDPOINT_ARGS "$GUEST_PIN" confirm
cp "$WORK/cr-out" "$WORK/publish.log"
echo "    --- cargo rivet fleet publish rivet-harness.apk --sync confirm exited $CR_RC ---"
show "$WORK/publish.log"
held_digests
# **The store was empty**, so what it holds now is what was sent — and the
# client's own report must name the same digest as sent.
DIGEST=$(head -1 "$WORK/held")
if [ "$CR_RC" -eq 0 ] && [ "$(wc -l <"$WORK/held" | tr -d ' ')" = 1 ] \
    && grep -qE "sent +$DIGEST" "$WORK/publish.log"; then
    ok "--sync confirm sent $DIGEST and $FLEETD's store holds it, read back with get file $OBJECTS"
else
    fail "the apk is not held after --sync confirm (exit $CR_RC, digest '$DIGEST', held: $(tr '\n' ' ' <"$WORK/held"))"
fi

restart_gantry
read_file "$APPS/gantry/data/last-artifacts.json"
if [ -n "$DIGEST" ] && grep -qF "{\"ok\":true,\"artifacts\":[{\"digest\":\"$DIGEST\",\"header\":\"" "$WORK/content"; then
    ok "the probe's next artifacts lists $DIGEST with its header"
else
    fail "the probe's artifacts does not list $DIGEST: $(head -c 300 "$WORK/content")"
fi

cp "$WORK/held" "$WORK/held-before"
NOAPK=$WORK/fleet-no-apk
cr fleet init --at "$NOAPK"
[ "$CR_RC" -eq 0 ] || { cat "$WORK/cr-out" >&2; die "fleet init (no-apk)"; }
# shellcheck disable=SC2086
cr fleet publish "$WORK/rivet-harness.apk" --signing-key "$WORK/no-apk.seed" --at "$NOAPK" \
    --sync $ENDPOINT_ARGS "$GUEST_PIN" confirm
cp "$WORK/cr-out" "$WORK/no-apk.log"
NOAPK_DIGEST=$(grep -oE 'refused +[0-9a-f]{64}' "$WORK/no-apk.log" | grep -oE '[0-9a-f]{64}' | head -1 || true)
held_digests
if [ "$CR_RC" -ne 0 ] && [ -n "$NOAPK_DIGEST" ] && [ "$NOAPK_DIGEST" != "$DIGEST" ] \
    && grep -q 'signed by the key labelled `throwaway-no-apk`' "$WORK/no-apk.log" \
    && [ "$(cat "$WORK/held-before")" = "$(cat "$WORK/held")" ]; then
    ok "an apk signed by a key whose mask lacks apk is refused at the door and not held (\"$(grep -o 'and that key is trusted to sign [a-z, ]*' "$WORK/no-apk.log" | head -1)\")"
else
    fail "the no-apk key's apk was not refused at the door (exit $CR_RC, refused '$NOAPK_DIGEST', held before: $(tr '\n' ' ' <"$WORK/held-before"), after: $(tr '\n' ' ' <"$WORK/held")):"
    show "$WORK/no-apk.log"
fi

# shellcheck disable=SC2086
cr install $ENDPOINT_ARGS "$GUEST_PIN" --artifact "$DIGEST" --at "$FLEET" confirm
cp "$WORK/cr-out" "$WORK/install.log"
journal
if [ "$CR_RC" -ne 0 ] && grep -aq 'an apk is held by a fleet server' "$WORK/install.log" "$WORK/journal" "$CONSOLE"; then
    ok "install of the apk artifact is refused by the guest's installer, by name ($(grep -aho 'an apk is held by a fleet server[^"]*' "$WORK/install.log" "$WORK/journal" "$CONSOLE" | head -1 | cut -c1-160))"
else
    fail "install of the apk artifact was not refused by name (exit $CR_RC):"
    show "$WORK/install.log"
fi

# ===========================================================================
echo
echo "--- 2b. fleet hide, unhide and retire, reaching the guest's store ---"
# ===========================================================================
# `ADR-052` §7. The same APK is published a second time under another product
# name, so the store holds two products and a prune can be seen to remove one
# and leave the other.
PRODUCT=$(sed -n 's/^product = "\([^"]*\)".*/\1/p' "$FLEET/manifest.toml" | head -1)
[ -n "$PRODUCT" ] || die "cannot read the published apk's product out of $FLEET/manifest.toml"
SECOND=$PRODUCT-second
# shellcheck disable=SC2086
cr fleet publish "$WORK/rivet-harness.apk" --product "$SECOND" --signing-key "$WORK/apk.seed" --at "$FLEET" \
    --sync $ENDPOINT_ARGS "$GUEST_PIN" confirm
cp "$WORK/cr-out" "$WORK/publish-second.log"
DIGEST2=$(grep -oE 'sent +[0-9a-f]{64}' "$WORK/publish-second.log" | grep -oE '[0-9a-f]{64}' | head -1 || true)
held_digests
if [ "$CR_RC" -eq 0 ] && [ -n "$DIGEST2" ] && [ "$DIGEST2" != "$DIGEST" ] \
    && [ "$(cat "$WORK/held")" = "$(printf '%s\n%s\n' "$DIGEST" "$DIGEST2" | sort)" ]; then
    ok "a second product, $SECOND, is published with --sync confirm; the store holds $DIGEST ($PRODUCT) and $DIGEST2 ($SECOND)"
else
    fail "the second product is not held beside the first (exit $CR_RC, second '$DIGEST2', held: $(tr '\n' ' ' <"$WORK/held")):"
    show "$WORK/publish-second.log"
fi

# `offered DIGEST` — whether the probe's last artifacts reply, in $WORK/content,
# answered and offers DIGEST.
offered() { grep -qF "\"digest\":\"$1\"" "$WORK/content"; }
answered() { grep -qF '{"ok":true,"artifacts":[' "$WORK/content"; }
# `push_and_sync LOG [--prune]` — `fleet push … confirm`, then `fleet sync …
# [--prune] confirm`; PUSH_RC, SYNC_RC, the sync's report in $WORK/LOG, and
# the store's listing in $WORK/held.
push_and_sync() {
    _log=$1
    shift
    # shellcheck disable=SC2086
    cr fleet push $ENDPOINT_ARGS "$GUEST_PIN" --at "$FLEET" confirm
    PUSH_RC=$CR_RC
    cp "$WORK/cr-out" "$WORK/$_log.push"
    # shellcheck disable=SC2086
    cr fleet sync $ENDPOINT_ARGS "$GUEST_PIN" --at "$FLEET" "$@" confirm
    SYNC_RC=$CR_RC
    cp "$WORK/cr-out" "$WORK/$_log"
    echo "    --- fleet push confirm exited $PUSH_RC; fleet sync ${*:+$* }confirm exited $SYNC_RC ---"
    show "$WORK/$_log"
    held_digests
}

cr fleet hide "$PRODUCT" --at "$FLEET"
HIDE_RC=$CR_RC
show "$WORK/cr-out"
if [ "$HIDE_RC" -eq 0 ] && grep -q '^\[\[hidden\]\]$' "$FLEET/manifest.toml" \
    && grep -qF "product = \"$PRODUCT\"" "$FLEET/manifest.toml" \
    && [ -z "$(git -C "$FLEET" status --porcelain -- manifest.toml)" ]; then
    ok "fleet hide $PRODUCT records it as [[hidden]] in the fleet repository's manifest and commits it"
else
    fail "fleet hide $PRODUCT (exit $HIDE_RC) did not commit a [[hidden]] entry:"
    show "$FLEET/manifest.toml"
fi

push_and_sync hide.log --prune
if [ "$PUSH_RC" -eq 0 ] && [ "$SYNC_RC" -eq 0 ] \
    && grep -qE "^  hidden +$DIGEST " "$WORK/hide.log" && ! grep -qE "^  sent +$DIGEST" "$WORK/hide.log"; then
    ok "fleet sync names $DIGEST as hidden and does not send it"
else
    fail "fleet sync did not report $DIGEST as hidden and unsent (push $PUSH_RC, sync $SYNC_RC)"
fi
PRUNED=$(grep -oE '^  pruned +[0-9a-f]{64}' "$WORK/hide.log" | grep -oE '[0-9a-f]{64}' | sort -u || true)
if [ "$PRUNED" = "$DIGEST" ] && [ "$(cat "$WORK/held")" = "$DIGEST2" ]; then
    ok "--prune confirm removed exactly the hidden product's object, $DIGEST, read back with get file $OBJECTS"
else
    fail "the prune removed '$(printf '%s' "$PRUNED" | tr '\n' ' ')', and the store holds: $(tr '\n' ' ' <"$WORK/held") (want $DIGEST2 alone)"
fi
if grep -qxF "$DIGEST2" "$WORK/held"; then
    ok "every other held object survives the prune: $DIGEST2 ($SECOND) is still held"
else
    fail "$DIGEST2 ($SECOND) did not survive the prune: $(tr '\n' ' ' <"$WORK/held")"
fi
restart_gantry
read_file "$APPS/gantry/data/last-artifacts.json"
if answered && ! offered "$DIGEST" && offered "$DIGEST2"; then
    ok "the probe's next artifacts no longer offers $PRODUCT, and still offers $SECOND"
else
    fail "after the hide the probe's artifacts is: $(head -c 400 "$WORK/content")"
fi

cr fleet unhide "$PRODUCT" --at "$FLEET"
UNHIDE_RC=$CR_RC
show "$WORK/cr-out"
push_and_sync unhide.log
if [ "$UNHIDE_RC" -eq 0 ] && ! grep -q '^\[\[hidden\]\]$' "$FLEET/manifest.toml" \
    && [ "$PUSH_RC" -eq 0 ] && [ "$SYNC_RC" -eq 0 ] && grep -qE "^  sent +$DIGEST" "$WORK/unhide.log" \
    && [ "$(cat "$WORK/held")" = "$(printf '%s\n%s\n' "$DIGEST" "$DIGEST2" | sort)" ]; then
    ok "fleet unhide $PRODUCT, push and sync sent $DIGEST again, and the store holds it beside $DIGEST2"
else
    fail "after unhide (exit $UNHIDE_RC, push $PUSH_RC, sync $SYNC_RC) the store holds: $(tr '\n' ' ' <"$WORK/held")"
fi
restart_gantry
read_file "$APPS/gantry/data/last-artifacts.json"
if answered && offered "$DIGEST" && offered "$DIGEST2"; then
    ok "the probe's next artifacts offers $PRODUCT again"
else
    fail "after the unhide the probe's artifacts is: $(head -c 400 "$WORK/content")"
fi

# The retire reads an apk product's signer with the application key, which is
# this run's throwaway one.
RIVET_APP_SEED=$WORK/apk.seed
export RIVET_APP_SEED
cr fleet retire "$SECOND" --kind apk --at "$FLEET"
RETIRE_RC=$CR_RC
unset RIVET_APP_SEED
show "$WORK/cr-out"
push_and_sync retire.log --prune
PRUNED=$(grep -oE '^  pruned +[0-9a-f]{64}' "$WORK/retire.log" | grep -oE '[0-9a-f]{64}' | sort -u || true)
if [ "$RETIRE_RC" -eq 0 ] && ! grep -qF "product = \"$SECOND\"" "$FLEET/manifest.toml" \
    && [ ! -e "$FLEET/objects/$DIGEST2" ] && [ "$PUSH_RC" -eq 0 ] && [ "$SYNC_RC" -eq 0 ] \
    && [ "$PRUNED" = "$DIGEST2" ] && [ "$(cat "$WORK/held")" = "$DIGEST" ]; then
    ok "fleet retire $SECOND removed it from the fleet repository, and push and --prune confirm removed exactly $DIGEST2 from the store"
else
    fail "after retire (exit $RETIRE_RC, push $PUSH_RC, sync $SYNC_RC) the prune removed '$(printf '%s' "$PRUNED" | tr '\n' ' ')' and the store holds: $(tr '\n' ' ' <"$WORK/held")"
fi
restart_gantry
read_file "$APPS/gantry/data/last-artifacts.json"
if answered && offered "$DIGEST" && ! offered "$DIGEST2"; then
    ok "the probe's next artifacts no longer offers the retired $SECOND, and still offers $PRODUCT"
else
    fail "after the retire the probe's artifacts is: $(head -c 400 "$WORK/content")"
fi

# ===========================================================================
echo
echo "--- 3. cargo rivet fleet phones list, rename, revoke ---"
# ===========================================================================
# shellcheck disable=SC2086
cr fleet phones list $ENDPOINT_ARGS "$GUEST_PIN"
show "$WORK/cr-out"
if [ "$CR_RC" -eq 0 ] && [ -n "$IDENTITY" ] && grep -qF "$IDENTITY  \"Hand-composed probe phone\"" "$WORK/cr-out"; then
    ok "fleet phones list shows the probe-enrolled phone"
else
    fail "fleet phones list does not show $IDENTITY (exit $CR_RC)"
fi

NEW_LABEL="Renamed by the operator"
# shellcheck disable=SC2086
cr fleet phones rename "$IDENTITY" "$NEW_LABEL" $ENDPOINT_ARGS "$GUEST_PIN" confirm
show "$WORK/cr-out"
RENAME_RC=$CR_RC
read_file "$PHONES/$IDENTITY.toml"
PHONE_UID=$(grep -oE '"uid": ?[0-9]+' "$WORK/content" | grep -oE '[0-9]+$' | head -1 || true)
PHONE_MODE=$(grep -oE '"mode": ?"[0-7]+"' "$WORK/content" | grep -oE '[0-7]+"$' | tr -d '"' | head -1 || true)
restart_gantry
read_file "$APPS/gantry/data/last-phones.json"
if [ "$RENAME_RC" -eq 0 ] && [ -n "$PHONE_UID" ] && [ "$PHONE_UID" = "$FLEETD_UID" ] \
    && [ "$PHONE_MODE" = 0600 ] && grep -qF "{\"ok\":true,\"phones\":[{\"identity\":\"$IDENTITY\",\"label\":\"$NEW_LABEL\"" "$WORK/content"; then
    ok "rename confirm changed the label and the probe's next phones shows \"$NEW_LABEL\" (the file is uid $PHONE_UID, mode $PHONE_MODE; $FLEETD is uid $FLEETD_UID)"
else
    fail "after rename (exit $RENAME_RC) the probe's phones is: $(cat "$WORK/content") (file uid $PHONE_UID mode $PHONE_MODE)"
fi

# shellcheck disable=SC2086
cr fleet phones revoke "$IDENTITY" $ENDPOINT_ARGS "$GUEST_PIN" confirm
show "$WORK/cr-out"
REVOKE_RC=$CR_RC
# shellcheck disable=SC2086
cr fleet phones list $ENDPOINT_ARGS "$GUEST_PIN"
LIST_AFTER=$(cat "$WORK/cr-out")
restart_gantry
read_file "$APPS/gantry/data/last-phones.json"
if [ "$REVOKE_RC" -eq 0 ] && grep -qF '{"ok":true,"phones":[]}' "$WORK/content" \
    && printf '%s' "$LIST_AFTER" | grep -q 'no phones are enrolled'; then
    ok "revoke confirm removed the phone: fleet phones list says none, and the probe's next phones is empty"
else
    fail "after revoke (exit $REVOKE_RC) list says '$LIST_AFTER' and the probe's phones is: $(cat "$WORK/content")"
fi

# ===========================================================================
echo
echo "--- 4. the proxy: plain: on port 80, app: on 443 ---"
# ===========================================================================
for _app in rivet-proxyd gantry; do
    # shellcheck disable=SC2086
    cr secrets "$_app" $ENDPOINT_ARGS "$GUEST_PIN" --authority "$WORK/ca" --name "$MACHINE" confirm
    [ "$CR_RC" -eq 0 ] || { cat "$WORK/cr-out" >&2; die "cargo rivet secrets $_app exited $CR_RC"; }
done
for _app in rivet-proxyd gantry; do
    ask apply service-restart "$_app" confirm
done

journal
PROXY_BOUND=$(grep -aho '"svc":"rivet-proxyd"[^}]*tcp and udp sockets are bound[^}]*' "$WORK/journal" "$CONSOLE" \
    | grep -o '"bound":"[^"]*"' | tail -1 || true)
if [ "$PROXY_BOUND" = '"bound":"0.0.0.0:443,0.0.0.0:80"' ]; then
    ok "the supervisor journals rivet-proxyd listening on 443 and 80: $PROXY_BOUND"
else
    fail "rivet-proxyd's bind record is not 443 and 80: '$PROXY_BOUND'"
fi

# `http PORT PATH [curl arguments…]` / `https …` — CODE is the status, 000 when
# nothing answered; the body is in $WORK/body.
http() {
    _port=$1 _path=$2
    shift 2
    : >"$WORK/body"
    set +e
    CODE=$(curl -sS --max-time 20 --resolve "$MACHINE:$_port:127.0.0.1" -o "$WORK/body" \
        -w '%{http_code}' "$@" "http://$MACHINE:$_port$_path" 2>"$WORK/curl-err")
    set -e
}
https() {
    _path=$1
    shift
    : >"$WORK/body"
    set +e
    CODE=$(curl -sS --max-time 20 --cacert "$WORK/ca/root.crt" --resolve "$MACHINE:$PROXY_PORT:127.0.0.1" \
        -o "$WORK/body" -w '%{http_code}' "$@" "https://$MACHINE:$PROXY_PORT$_path" 2>"$WORK/curl-err")
    set -e
}

TOKEN="Bearer hand-composed.token-value/with+symbols="
ATTEMPT=1
while :; do
    https /gantry/api/echo -H "Authorization: $TOKEN"
    [ "$CODE" = 200 ] && break
    [ "$ATTEMPT" -ge 30 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$CODE" = 200 ] && [ "$(cat "$WORK/body")" = "authorization=$TOKEN" ]; then
    ok "on 443 the app: route is forwarded with no client certificate and the probe received Authorization unchanged ($(cat "$WORK/body"))"
else
    fail "the app: route on 443: $CODE, body '$(cat "$WORK/body")', $(cat "$WORK/curl-err")"
fi

http "$PLAIN_PORT" /gantry/install
if [ "$CODE" = 200 ] && grep -q 'gantry-probe install page, GET /install' "$WORK/body"; then
    ok "GET /gantry/install over plain HTTP on 80 is 200, served by the probe"
else
    fail "GET /gantry/install on 80: $CODE $(cat "$WORK/body" "$WORK/curl-err")"
fi
http "$PLAIN_PORT" /gantry/install -X POST --data-binary 'a write'
if [ "$CODE" = 405 ]; then
    ok "POST /gantry/install on 80 is 405 (the probe would have answered 200)"
else
    fail "POST /gantry/install on 80: $CODE $(cat "$WORK/body")"
fi
http "$PLAIN_PORT" /gantry/api/echo
if [ "$CODE" = 404 ]; then
    ok "a non-plain path, /gantry/api/echo, on 80 is 404"
else
    fail "/gantry/api/echo on 80: $CODE $(cat "$WORK/body")"
fi
http "$PLAIN_PORT" /
if [ "$CODE" = 404 ]; then
    ok "/ on 80 is 404, not the index and not a redirect"
else
    fail "/ on 80: $CODE $(cat "$WORK/body")"
fi

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true
QEMU_PID=

# ===========================================================================
echo
echo "--- 4b. BOOT 2: no plain rule, nothing on 80 ---"
# ===========================================================================
render_manifest gantry "${DAEMON}listen = [\"tcp:8080\"]
proxy-path = \"gantry\"
proxy-auth-paths = [\"app:api\"]
"
sign gantry "$WORK/gantry"
DOC2=$WORK/machine-2.toml
{
    printf 'version = 1\nhostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for _app in rivet-proxyd gantry; do
        printf '\n[services.%s]\nenabled = true\n' "$_app"
    done
    printf '\n[applications.rivet-proxyd]\nupstream-name = "%s"\n' "$MACHINE"
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC2"
CONSOLE=$WORK/console-2.log
boot "$CONSOLE" --document "$DOC2" --app "gantry:$WORK/gantry:$WORK/gantry.toml:$WORK/gantry.header"
# shellcheck disable=SC2086
cr secrets rivet-proxyd $ENDPOINT_ARGS "$GUEST_PIN" --authority "$WORK/ca" --name "$MACHINE" confirm
[ "$CR_RC" -eq 0 ] || { cat "$WORK/cr-out" >&2; die "cargo rivet secrets rivet-proxyd exited $CR_RC (boot 2)"; }
ask apply service-restart rivet-proxyd confirm
ATTEMPT=1
while :; do
    https / --cert "$WORK/ca/browser.crt" --key "$WORK/ca/browser.key"
    [ "$CODE" = 200 ] && break
    [ "$ATTEMPT" -ge 30 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$CODE" = 200 ]; then
    ok "in the same boot the proxy serves the operator's index on 443 (200), so its forward is live"
else
    fail "the proxy did not serve the index on 443 in boot 2: $CODE $(cat "$WORK/curl-err")"
fi
ATTEMPT=1
PROXY_BOUND=
while :; do
    journal
    PROXY_BOUND=$(grep -aho '"svc":"rivet-proxyd"[^}]*tcp and udp sockets are bound[^}]*' "$WORK/journal" "$CONSOLE" \
        | grep -o '"bound":"[^"]*"' | tail -1 || true)
    [ -n "$PROXY_BOUND" ] && break
    [ "$ATTEMPT" -ge 30 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ "$PROXY_BOUND" = '"bound":"0.0.0.0:443"' ]; then
    ok "with no plain rule the supervisor journals rivet-proxyd on 443 alone: $PROXY_BOUND"
else
    fail "with no plain rule rivet-proxyd's bind record is '$PROXY_BOUND'"
fi
http "$PLAIN_PORT" /gantry/install
if [ "$CODE" = 000 ]; then
    ok "the forwarded port 80 answers nothing ($(tr -d '\n' <"$WORK/curl-err" | cut -c1-120))"
else
    fail "port 80 answered $CODE with no plain rule: $(cat "$WORK/body")"
fi

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true
QEMU_PID=

# ===========================================================================
echo
echo "--- 5. BOOT 3: gantry and not-gantry both claim the gantry socket ---"
# ===========================================================================
# The gantry socket is not shared, so two applications claiming it is a
# contest, and the supervisor refuses it to EVERY claimant: granting it to
# whichever sorts first would let an application installed later take the
# socket from the one holding it, just by its name. Boot 1 is the control —
# the same gantry, alone, was granted the socket and answered.
render_manifest gantry "$GANTRY_META"
sign gantry "$WORK/gantry"
DOC3=$WORK/machine-3.toml
{
    printf 'version = 1\nhostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for _app in "$FLEETD" gantry not-gantry; do
        printf '\n[services.%s]\nenabled = true\n' "$_app"
    done
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC3"
CONSOLE=$WORK/console-3.log
boot "$CONSOLE" \
    --state "$SEEDED_STATE" \
    --document "$DOC3" \
    --app "$FLEETD:$WORK/$FLEETD:$WORK/$FLEETD.toml:$WORK/$FLEETD.header" \
    --app "gantry:$WORK/gantry:$WORK/gantry.toml:$WORK/gantry.header" \
    --app "not-gantry:$WORK/not-gantry:$WORK/not-gantry.toml:$WORK/not-gantry.header"

CLAIMANTS="and so does every application in [gantry, not-gantry]"
# "gantry wants …" is also a substring of "not-gantry wants …", so gantry's own
# refusal is read from the record whose `svc` names it, and its detail must
# START with gantry's name.
own_refusal() {
    grep -ahoE "\"svc\":\"$1\"[^}]*declares connects and the connection was refused[^}]*" \
        "$WORK/journal" "$CONSOLE" | grep -F "\"$1 wants $FLEETD:gantry, $CLAIMANTS" | head -1 || true
}
ATTEMPT=1
while :; do
    journal
    GANTRY_OWN=$(own_refusal gantry)
    NOT_GANTRY_OWN=$(own_refusal not-gantry)
    [ -n "$GANTRY_OWN" ] && [ -n "$NOT_GANTRY_OWN" ] && break
    [ "$ATTEMPT" -ge 30 ] && break
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
if [ -n "$GANTRY_OWN" ]; then
    ok "the supervisor refused gantry's connects and named both claimants (\"gantry wants $FLEETD:gantry, $CLAIMANTS\")"
else
    fail "gantry was not refused with both claimants named"
    grep -a '"svc":"gantry"' "$WORK/journal" | tail -5 | cut -c1-400
fi
if [ -n "$NOT_GANTRY_OWN" ]; then
    ok "the supervisor refused not-gantry's connects and named both claimants (\"not-gantry wants $FLEETD:gantry, $CLAIMANTS\")"
else
    fail "not-gantry was not refused with both claimants named"
    grep -a '"svc":"not-gantry"' "$WORK/journal" | tail -5 | cut -c1-400
fi
done_of gantry
GANTRY_DONE=$DONE
done_of not-gantry
if [ "$GANTRY_DONE" = 0 ] && [ "$DONE" = 0 ]; then
    ok "neither claimant ran a round: neither was started"
else
    fail "a refused claimant ran (gantry rounds $GANTRY_DONE, not-gantry rounds $DONE)"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "$NAME: PASS"
else
    echo "$NAME: FAIL — see the lines above" >&2
    mkdir -p "$REPO/out" 2>/dev/null || true
    cp "$WORK/console-1.log" "$REPO/out/$NAME.console-1.log" 2>/dev/null || true
    cp "$WORK/console-2.log" "$REPO/out/$NAME.console-2.log" 2>/dev/null || true
    cp "$WORK/console-3.log" "$REPO/out/$NAME.console-3.log" 2>/dev/null || true
fi
exit "$VERDICT"
