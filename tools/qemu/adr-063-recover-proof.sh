#!/bin/sh
# tools/qemu/adr-063-recover-proof.sh — **a machine is backed up, destroyed,
# and recovered by `cargo rivet recover --after-boot` with the same pin, the
# same application and byte-identical data — over the real control plane.**
#
# `ADR-063` §7, `.rulings/` R-235. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-063-recover-proof.sh
#
# ## The machine, before
#
# One guest booted with a throwaway identity, `peers` naming a throwaway
# operator, and one extra secret (`probe.psk`). Its document enables the
# image's `rivet-proxyd` and `restore-probe` (`tools/qemu/restore-probe`),
# which is packaged by `cargo rivet package` with a throwaway key the guest
# trusts, published into a scratch fleet repository, and installed by digest
# with `cargo rivet install --artifact`. Its TLS material comes from a
# throwaway fleet authority through `cargo rivet secrets`. Its data is then
# changed, so the backup holds something a fresh install does not.
#
# It is backed up twice: `cargo rivet backup` (the data) and `cargo rivet
# backup-secrets` (the secrets, the passphrase typed into a pty).
#
# ## Destroyed
#
# The guest is stopped and booted again with a FRESH `/state`: the identity
# files and `peers` a card written by `provision --existing` carries, and
# nothing else — no `probe.psk`, no application, no data. The card write
# itself is not exercised here: it is a person's, and `recover` refuses it
# without a terminal, which is asserted.
#
# ## Recovered
#
# * the plan refuses a host the register does not record when no pin is
#   given, and data for an application the document does not install;
# * the plan with `--pin` names the digest and writes nothing;
# * `confirm` without `--after-boot` and without a terminal is refused;
# * `--after-boot`, the passphrase typed: the secrets are restored and the
#   application installed by digest, and step 4 is REFUSED — the application
#   cannot answer without its material, which is in no backup — and nothing
#   is sent;
# * the material is staged with `cargo rivet secrets`, as the install's
#   printed steps say; `--after-boot` again skips what is done and REFUSES to
#   write over the fresh application's data without `--replace`, and sends
#   nothing — a backup afterwards is still the fresh data;
# * `--after-boot --replace restore-probe` restores it.
#
# Then: every connection pins the original pin; a `restore-secrets` plan of
# the original backup names all four files `identical, skipped`; the store
# holds the published digest; and a new `cargo rivet backup` is byte-identical
# to the one taken before the machine was destroyed. The script exits non-zero
# on any mismatch.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column: no Raspberry Pi, no SD card, no card write
# and no reboot after the restore, so that the machine's programs start from
# the restored secrets is not observed. `/state` is the initramfs. The
# application is a fixture. The terminal is `script(1)`'s pty. Every key and
# the passphrase are THROWAWAY, minted into the container's temporary
# directory on every run; no key of anybody's is read.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release
HOST_OUT=$TARGET_DIR/release

WORK=${TMPDIR:-/tmp}/adr-063-recover-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-063-recover-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-063-recover-proof: $1" >&2
    exit 1
}
ok() {
    echo "  OK   $1"
}

command -v script >/dev/null || die "no script(1) — this proof types passphrases into a pty it gives the command"
command -v mkfifo >/dev/null || die "no mkfifo"
command -v openssl >/dev/null || die "no openssl to mint a throwaway key with"
command -v curl >/dev/null || die "no curl to change the application's data with"

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
PEERS=${MCPD_PEERS_PATH##*/}
CONTROL_PORT=$(number "$REPO/rivet-config/src/control_plane.rs" DEFAULT_PORT)
ARTIFACTS=$(constant "$REPO/rivet-core/src/state.rs" ARTIFACTS)
PROXY_PORT=8443
GUEST=recover-guest
SUFFIX=local
MACHINE=$GUEST.$SUFFIX
APP=restore-probe

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."
STRIP=$(dirname "$CC_aarch64_unknown_linux_musl")/aarch64-linux-strip
[ -x "$STRIP" ] || die "no $STRIP to strip rivet-proxyd with"

echo "adr-063-recover-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd \
    -p rivetos-proxyd -p rivetos-deploy ) || die "the guest's binaries did not build"
( cd "$HERE/restore-probe" && cargo build --release --target "$TARGET" ) \
    || die "tools/qemu/restore-probe did not build"
[ -f "$OUT/$APP" ] || die "the probe was not built at $OUT/$APP"
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision ) \
    || die "the workstation tools did not build"
CARGO_RIVET=$HOST_OUT/cargo-rivet
PROVISION=$HOST_OUT/rivet-provision
( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer
[ -x "$SIGNER" ] || die "no fixture signer at $SIGNER"
"$STRIP" -o "$WORK/rivet-proxyd" "$OUT/rivet-proxyd" || die "rivet-proxyd could not be stripped"

# **From here, nothing may read this host's ~/.rivet, its git configuration or
# a real seed.** Set after the build, which needs the real toolchain.
HOME=$WORK/home
mkdir -p "$HOME"
export HOME
GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_NOSYSTEM
unset RIVET_SYSTEM_SEED RIVET_FLEET_REPOSITORY 2>/dev/null || true

# ---------------------------------------------------------------------------
# Throwaway keys, identities and authority
# ---------------------------------------------------------------------------
echo "adr-063-recover-proof: minting THROWAWAY keys, identities and an authority in $WORK"
KEY=$WORK/throwaway-app.hex
openssl rand -hex 32 >"$KEY" || die "could not mint a throwaway key"
PUB=$WORK/throwaway-app.pub
# `install` reads the application seed to plan the steps that remain.
RIVET_APP_SEED=$KEY
export RIVET_APP_SEED

mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$1" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; die "no $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
PIN=$(mint guest)
OPERATOR_PIN=$(mint operator)
[ -n "$PIN" ] && [ -n "$OPERATOR_PIN" ] || die "could not read a public key"

# What a card written by `provision --existing` carries: the identity and peers.
card_secrets() {
    rm -rf "$1"
    mkdir -p "$1"
    cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$1/"
    printf '# the operator\n%s\n' "$OPERATOR_PIN" >"$1/$PEERS"
}
card_secrets "$WORK/secrets-before"
PSK_VALUE=7265636f7665722d70726f6f662d70736b
printf '%s' "$PSK_VALUE" >"$WORK/secrets-before/probe.psk"

"$PROVISION" ca "$SUFFIX" restore-fleet "$WORK/ca" operator --no-install \
    >"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no authority"; }
"$PROVISION" ca server "$GUEST" "$WORK/ca" \
    >>"$WORK/ca.log" 2>&1 </dev/null || { cat "$WORK/ca.log" >&2; die "no server pair"; }

PASSPHRASE='adr-063 recover proof passphrase, throwaway'

# ---------------------------------------------------------------------------
# The application, packaged and published by digest
# ---------------------------------------------------------------------------
CRATE=$WORK/probe-crate
mkdir -p "$CRATE"
{
    sed -e 's/\r$//' "$HERE/restore-probe/Cargo.toml"
    printf '\n[package.metadata.rivet]\nkind = "daemon"\nrestart = "on-failure"\nready = "none"\n'
    printf 'network = "none"\nstate = "private"\nlisten = ["tcp:8080"]\n'
    printf 'proxy-path = "probe"\nbackup-path = "api/v1/backup"\nbackup-format = "sqlite"\n'
    printf 'restore-path = "api/v1/restore"\n'
} >"$CRATE/Cargo.toml"
# **No `material` key, on purpose.** An application declaring material that its
# enabled document puts into service is refused by `install` until the material
# is staged, and staging needs the data directory only an install makes. A
# card carries its document's applications (`cmd/cargo-rivet/src/carry.rs`),
# which is what is expected to make that directory on a board — unproven here,
# because this harness has no card. So the fixture declares nothing, installs,
# and is refused at step 4 instead, for not answering.
"$CARGO_RIVET" package --manifest-path "$CRATE/Cargo.toml" --target "$TARGET" \
    --out-dir "$WORK/rvt" --no-assets --signing-key "$KEY" --public-key-out "$PUB" \
    --binary "$OUT/$APP" >"$WORK/package.log" 2>&1 \
    || { cat "$WORK/package.log" >&2; die "cargo rivet package failed for $APP"; }
RVT=$(find "$WORK/rvt" -name '*.rvt' -type f | head -1)
[ -n "$RVT" ] || die "no .rvt was written"
"$SIGNER" trust-store "$WORK/trust" "throwaway:application:$PUB" >/dev/null \
    || die "could not write the trust store"

FLEET=$WORK/fleet
"$CARGO_RIVET" fleet init --at "$FLEET" >"$WORK/init.log" 2>&1 || { cat "$WORK/init.log" >&2; die "fleet init"; }
"$CARGO_RIVET" fleet publish "$RVT" --trusted-key "$PUB" --at "$FLEET" >"$WORK/publish.log" 2>&1 \
    || { cat "$WORK/publish.log" >&2; die "fleet publish"; }
DIGEST=$(grep -oE '[0-9a-f]{64}' "$WORK/publish.log" | head -1)
[ -n "$DIGEST" ] && [ -f "$FLEET/objects/$DIGEST" ] || die "fleet publish left no object"

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "%s"\n' "$GUEST"
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    for app in rivet-proxyd "$APP"; do
        printf '\n[services.%s]\nenabled = true\n' "$app"
    done
    printf '\n[applications.rivet-proxyd]\nupstream-name = "%s"\n' "$MACHINE"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"
# The document a verified card write records, committed as that write would.
mkdir -p "$FLEET/documents"
cp "$DOC" "$FLEET/documents/$GUEST.provisioning.toml"
git -C "$FLEET" add documents >/dev/null
git -C "$FLEET" -c user.name=proof -c user.email=proof@rivet.invalid commit -q -m "record $GUEST" \
    || die "could not record the document"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

# ---------------------------------------------------------------------------
# Harness
# ---------------------------------------------------------------------------
QEMU=
boot() {
    LOG=$WORK/console-$1.log
    echo "adr-063-recover-proof: booting $1"
    "$HERE/run.sh" "$OUT/rivet-initd" \
        --supervisor "$OUT/rivet-sysd" \
        --cli "$OUT/rivet" \
        --netd "$OUT/rivet-netd" \
        --mcpd "$OUT/rivet-mcpd" \
        --deploy "$OUT/rivet-deploy" \
        --proxyd "$WORK/rivet-proxyd" \
        --secrets "$2" \
        --trust "$WORK/trust" \
        --document "$DOC" \
        --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" --forward-port "$PROXY_PORT:443" \
        >"$LOG" 2>&1 </dev/null &
    QEMU=$!
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
    wait_for_answers
}
halt() {
    [ -n "$QEMU" ] || return 0
    kill "$QEMU" 2>/dev/null || true
    wait "$QEMU" 2>/dev/null || true
    QEMU=
}
trap halt EXIT

# `cargo rivet …` with NO terminal. Sets RC.
cr() {
    set +e
    "$CARGO_RIVET" "$@" >"$WORK/out" 2>"$WORK/err" </dev/null
    RC=$?
    set -e
}
quote() {
    for word in "$@"; do
        printf "'%s' " "$(printf '%s' "$word" | sed "s/'/'\\\\''/g")"
    done
}
# `cargo rivet …` under a REAL pty, typing the passphrase at every prompt.
typed() {
    rm -f "$WORK/tty-in"
    mkfifo "$WORK/tty-in"
    : >"$WORK/out"
    COMMAND=$(quote "$CARGO_RIVET" "$@")
    set +e
    timeout 900 script -q -e -c "$COMMAND" /dev/null <"$WORK/tty-in" >"$WORK/out" 2>&1 &
    PTY=$!
    exec 9>"$WORK/tty-in"
    TYPED=0
    while kill -0 "$PTY" 2>/dev/null; do
        PROMPTS=$(grep -o -e 'passphrase for this secrets backup:' -e 'type it again to confirm:' \
            "$WORK/out" 2>/dev/null | wc -l)
        if [ "$PROMPTS" -gt "$TYPED" ]; then
            sleep 0.5
            printf '%s\n' "$PASSPHRASE" >&9
            TYPED=$((TYPED + 1))
        fi
        sleep 0.2
    done
    wait "$PTY"
    RC=$?
    exec 9>&-
    set -e
    cp "$WORK/out" "$WORK/err"
    if grep -qF "$PASSPHRASE" "$WORK/out"; then
        fail "the typed passphrase was echoed"
    fi
}
endpoint() {
    printf '%s\n' --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/operator" --pin "$PIN"
}
rivet() {
    # shellcheck disable=SC2046
    cr run $(endpoint) -- "$@"
}
wait_for_answers() {
    ATTEMPT=1
    while :; do
        rivet get status
        [ "$RC" -eq 0 ] && break
        [ "$ATTEMPT" -ge 20 ] && break
        ATTEMPT=$((ATTEMPT + 1))
        sleep 2
    done
    [ "$RC" -eq 0 ] || { cat "$WORK/err" >&2; die "\`get status\` exited $RC"; }
}
bytes_of() {
    od -An -v -tx1 "$1" | tr -d ' \n'
}
# Stage the TLS material each routed application needs, and restart both.
stage_material() {
    for app in rivet-proxyd "$APP"; do
        # shellcheck disable=SC2046
        cr secrets "$app" $(endpoint) --authority "$WORK/ca" --name "$MACHINE" confirm
        [ "$RC" -eq 0 ] || die "cargo rivet secrets $app exited $RC: $(cat "$WORK/out" "$WORK/err")"
    done
    for app in rivet-proxyd "$APP"; do
        rivet apply service-restart "$app" confirm
        [ "$RC" -eq 0 ] || rivet apply service-start "$app" confirm
    done
}
# A backup of the application into $1, retried while it comes up. Sets FILE.
backup_into() {
    ATTEMPT=1
    while :; do
        rm -rf "$1"
        # shellcheck disable=SC2046
        cr backup $(endpoint) --service "$APP" --into "$1" confirm
        [ "$RC" -eq 0 ] && break
        [ "$ATTEMPT" -ge 40 ] && break
        ATTEMPT=$((ATTEMPT + 1))
        sleep 2
    done
    FILE=$(ls "$1/$APP/"* 2>/dev/null | head -1 || true)
}

# ===========================================================================
# The machine, before
# ===========================================================================
boot before "$WORK/secrets-before"

echo
echo "--- the machine as it was: install by digest, material, changed data ---"
# shellcheck disable=SC2046
cr install $(endpoint) --artifact "$DIGEST" --at "$FLEET" confirm
[ "$RC" -eq 0 ] || die "the first install exited $RC: $(cat "$WORK/out" "$WORK/err")"
ok "$APP installed by digest $DIGEST"
stage_material
backup_into "$WORK/fresh-before"
[ "$RC" -eq 0 ] && [ -n "$FILE" ] || die "$APP never answered its backup route: $(cat "$WORK/out" "$WORK/err")"
FRESH=$FILE
curl -sS --fail --cert "$WORK/ca/browser.crt" --key "$WORK/ca/browser.key" \
    --cacert "$WORK/ca/root.crt" --resolve "$MACHINE:$PROXY_PORT:127.0.0.1" \
    -X POST --data-binary '' "https://$MACHINE:$PROXY_PORT/probe/api/v1/mutate" >"$WORK/out" 2>"$WORK/err" \
    || die "the mutate route did not answer: $(cat "$WORK/out" "$WORK/err")"

echo
echo "--- backed up: the data, and the secrets with a typed passphrase ---"
backup_into "$WORK/backups"
[ "$RC" -eq 0 ] && [ -n "$FILE" ] || die "the backup did not arrive: $(cat "$WORK/out" "$WORK/err")"
ORIGINAL=$FILE
if [ "$(bytes_of "$ORIGINAL")" != "$(bytes_of "$FRESH")" ]; then
    ok "the backup holds the changed data ($(wc -c <"$ORIGINAL" | tr -d ' ') bytes), not what a fresh install writes"
else
    die "the data did not change, so a fresh install would already hold it and nothing would be proven"
fi
# shellcheck disable=SC2046
typed backup-secrets $(endpoint) --into "$WORK/secrets-backup" confirm
[ "$RC" -eq 0 ] || die "backup-secrets exited $RC: $(cat "$WORK/out")"
ok "the secrets are sealed in $WORK/secrets-backup"
halt

# ===========================================================================
# Destroyed: a fresh /state, holding only what a card carries
# ===========================================================================
card_secrets "$WORK/secrets-card"
boot destroyed "$WORK/secrets-card"
rivet get file "/state/secrets/probe.psk"
grep -q '"present":false' "$WORK/out" && ok "the destroyed machine has no probe.psk" \
    || fail "the destroyed machine still holds probe.psk: $(cat "$WORK/out")"
rivet get file "$ARTIFACTS/objects/$DIGEST"
grep -q '"present":false' "$WORK/out" && ok "and no $APP artifact" \
    || fail "the destroyed machine still holds the artifact: $(cat "$WORK/out")"

RECOVER="recover $GUEST --from $WORK/backups --secrets $WORK/secrets-backup --at $FLEET"

echo
echo "--- the plan ---"
# shellcheck disable=SC2086
cr $RECOVER
if [ "$RC" -ne 0 ] && grep -q 'NOT IN THE REGISTER' "$WORK/err"; then
    ok "with no register record and no --pin, refused"
else
    fail "a host with no pin was not refused: $(cat "$WORK/out" "$WORK/err")"
fi
mkdir -p "$WORK/stray/$APP" "$WORK/stray/tempmon"
cp "$ORIGINAL" "$WORK/stray/$APP/"
cp "$ORIGINAL" "$WORK/stray/tempmon/"
# shellcheck disable=SC2046
cr recover "$GUEST" --from "$WORK/stray" --at "$FLEET" $(endpoint)
if [ "$RC" -ne 0 ] && grep -q 'does not install' "$WORK/err"; then
    ok "data for an application the document does not install, refused"
else
    fail "stray application data was not refused: $(cat "$WORK/out" "$WORK/err")"
fi
# shellcheck disable=SC2046,SC2086
cr $RECOVER $(endpoint)
if [ "$RC" -eq 0 ] && grep -q "$DIGEST" "$WORK/out" && grep -q 'Nothing was written' "$WORK/out"; then
    ok "with --pin, a plan naming the digest, and nothing written"
else
    fail "the plan was not a plan: $(cat "$WORK/out" "$WORK/err")"
fi
# shellcheck disable=SC2046,SC2086
cr $RECOVER $(endpoint) confirm
if [ "$RC" -ne 0 ] && grep -q 'written by a person at a terminal' "$WORK/err"; then
    ok "the card step, with no terminal, refused"
else
    fail "the card step ran without a terminal: $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- after boot: secrets and install; the data refused without material ---"
# shellcheck disable=SC2046,SC2086
typed $RECOVER $(endpoint) --after-boot confirm
if [ "$RC" -ne 0 ] && grep -q 'probe.psk' "$WORK/out" && grep -q 'read back identical' "$WORK/out" \
    && grep -q 'STOPPED AT STEP 4' "$WORK/out" && grep -q 'NO APPLICATION WAS SENT ANY DATA' "$WORK/out"; then
    ok "secrets restored, $APP installed, and step 4 refused, sending nothing"
else
    fail "the first --after-boot was not that: $(cat "$WORK/out")"
fi
rivet get file "$ARTIFACTS/objects/$DIGEST"
grep -q '"present":true' "$WORK/out" && ok "the store holds the published digest" \
    || fail "the store does not hold $DIGEST: $(cat "$WORK/out")"

echo
echo "--- the material staged; the fresh data is not written over unasked ---"
stage_material
backup_into "$WORK/fresh-after"
[ "$RC" -eq 0 ] && [ -n "$FILE" ] || die "$APP never answered after its material: $(cat "$WORK/out" "$WORK/err")"
FRESH_AFTER=$FILE
# shellcheck disable=SC2046,SC2086
typed $RECOVER $(endpoint) --after-boot confirm
if [ "$RC" -ne 0 ] && grep -q 'identical on the board' "$WORK/out" \
    && grep -q "replace $APP" "$WORK/out" && grep -q 'NO APPLICATION WAS SENT ANY DATA' "$WORK/out"; then
    ok "secrets skipped as identical, and a restore over the fresh data refused without --replace"
else
    fail "the second --after-boot did not refuse to write over data: $(cat "$WORK/out")"
fi
backup_into "$WORK/still-fresh"
if [ "$RC" -eq 0 ] && [ "$(bytes_of "$FILE")" = "$(bytes_of "$FRESH_AFTER")" ]; then
    ok "and nothing was sent: $APP still holds its fresh data"
else
    fail "the refused restore changed the data"
fi

echo
echo "--- --replace $APP ---"
# shellcheck disable=SC2046,SC2086
typed $RECOVER $(endpoint) --after-boot --replace "$APP" confirm
if [ "$RC" -eq 0 ] && grep -q 'every step finished' "$WORK/out" && grep -q "$APP answered 200" "$WORK/out"; then
    ok "recovered: $APP answered 200"
else
    fail "the recovery with --replace did not finish: $(cat "$WORK/out")"
fi

echo
echo "--- the same machine ---"
backup_into "$WORK/after"
if [ "$RC" -eq 0 ] && [ "$(bytes_of "$FILE")" = "$(bytes_of "$ORIGINAL")" ]; then
    ok "a new backup is byte-identical to the one taken before the machine was destroyed"
else
    fail "the data after recovery is not the original: $(cat "$WORK/out" "$WORK/err")"
fi
# shellcheck disable=SC2046
typed restore-secrets $(endpoint) --from "$WORK/secrets-backup"
if [ "$RC" -eq 0 ] && [ "$(grep -c 'identical, skipped' "$WORK/out")" -eq 4 ] \
    && ! grep -qE '  (write|REPLACE)' "$WORK/out"; then
    ok "the original secrets backup plans all four files identical: the same identity and pin"
else
    fail "the secrets are not the original's: $(cat "$WORK/out")"
fi
rivet get status
if grep -q "\"name\":\"$APP\"" "$WORK/out"; then
    ok "$APP is a service on the recovered machine, reached by the original pin"
else
    fail "$APP is not on the recovered machine: $(cat "$WORK/out")"
fi

halt
echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-063-recover-proof: PASS — destroyed and recovered: the same pin, the same application, byte-identical data"
else
    echo "adr-063-recover-proof: FAIL — the console logs are in $WORK" >&2
fi
exit "$VERDICT"
