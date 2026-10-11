#!/bin/sh
# tools/qemu/adr-030-secrets-proof.sh — **a booted guest's /state/secrets is
# pulled to the workstation and sealed under a typed passphrase, put back onto
# the same guest, and refused on a guest with another identity — over the real
# control plane.**
#
# `ADR-030` §4 and §5, `.rulings/` R-183 D5 and R-185. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-030-secrets-proof.sh
#
# ## How a passphrase is typed with no terminal
#
# `cargo rivet backup-secrets` and `restore-secrets` read the passphrase from a
# real terminal and nowhere else: there is no flag, no environment variable and
# no test hook in the binary. So this runs each of them under `script(1)`
# (util-linux, in the container), which gives the command a real pty, and types
# into that pty through a FIFO — waiting until the prompt has appeared before it
# types, as a person would. The binary under test is the release build, unchanged.
#
# ## What it asserts
#
# Guest A, booted with a device identity, a peers file and one extra secret
# (`probe.psk`):
#
# * the unconfirmed `backup-secrets` is a plan that reads no secret;
# * the confirmed backup, with no terminal, is REFUSED and writes nothing;
# * the confirmed backup under a pty, passphrase typed twice, writes ONE file,
#   `secrets-backup.enc`, beginning `RVTSECR`, in which neither `probe.psk`'s
#   value nor its name is readable — and the passphrase was not echoed;
# * an unconfirmed restore of it onto the untouched guest names all four files
#   `identical, skipped` — the backup holds the guest's bytes exactly;
# * `apply restore-secret peers` with a peers file that does not name this
#   operator is REFUSED by the guest, E_UNSAFE — the lock-out guard, board side;
# * after `probe.psk` is removed on the guest and `device.crt` is overwritten,
#   `restore-secrets` without `--replace` is refused and writes nothing, and
#   with `--replace` it writes both back — and a restore plan afterwards names
#   all four files `identical, skipped` again;
# * a wrong passphrase, and a copy of the backup with one byte of the envelope
#   changed, are each refused as `wrong passphrase, or this file has been
#   altered`, with NOTHING WAS SENT;
# * a directory in the unencrypted layout an earlier build wrote is refused,
#   naming it, before any passphrase is asked for.
#
# Guest B, booted with ANOTHER device identity and the same operator:
#
# * `restore-secrets` of guest A's backup, with guest B's pin, is refused as a
#   DIFFERENT BOARD once it opens, and a backup of guest B taken beforehand
#   still plans as `identical, skipped` afterwards — guest B's store is unchanged.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column: no Raspberry Pi, no SD card, no reboot
# after the restore — so that `rivet-mcpd` starts from the restored files is
# not observed here. `/state` is the initramfs, not a card's partition. The
# terminal is `script(1)`'s pty, not a person's. The identities and the
# passphrase are THROWAWAY, minted into the container's temporary directory on
# every run; no key of anybody's is read.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-030-secrets-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-030-secrets-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-030-secrets-proof: $1" >&2
    exit 1
}
ok() {
    echo "  OK   $1"
}

command -v script >/dev/null || die "no script(1) — this proof types passphrases into a pty it gives the command"
command -v mkfifo >/dev/null || die "no mkfifo"

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
STORE=$(constant "$REPO/rivet-core/src/state.rs" SECRETS)
CONTROL_PORT=$(number "$REPO/rivet-config/src/control_plane.rs" DEFAULT_PORT)
RESTORE=$(constant "$REPO/rivet-core/src/operation/restore_secret.rs" SUBJECT)
BACKUP_FILE=$(constant "$REPO/cmd/cargo-rivet/src/machine_secrets.rs" BACKUP_FILE)
UNENCRYPTED_MANIFEST=$(constant "$REPO/cmd/cargo-rivet/src/machine_secrets.rs" UNENCRYPTED_MANIFEST)

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-030-secrets-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd ) \
    || die "the guest's binaries did not build"
( cd "$REPO" && cargo build --release -p cargo-rivet ) || die "cargo-rivet did not build"
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet

# ---------------------------------------------------------------------------
# Throwaway identities: two guests, one operator
# ---------------------------------------------------------------------------
echo "adr-030-secrets-proof: minting THROWAWAY identities in $WORK"
mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$1" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; die "no $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
PIN_A=$(mint guest-a)
PIN_B=$(mint guest-b)
OPERATOR_PIN=$(mint operator)
[ -n "$PIN_A" ] && [ -n "$PIN_B" ] && [ -n "$OPERATOR_PIN" ] || die "could not read a public key"
[ "$PIN_A" != "$PIN_B" ] || die "the two guests minted the same key"

# A throwaway passphrase, typed into a pty; never a flag, never the environment
# of the command under test.
PASSPHRASE='adr-030 proof passphrase, throwaway'
WRONG='not the passphrase that sealed it'

# A store as a board holds one: the identity, the peers file, and a value in
# hex as `rivet_core::secret` stores it.
PSK_VALUE=70726f62652d7073652d76616c7565
store_for() {
    mkdir -p "$WORK/secrets-$1"
    cp "$WORK/$1/device.seed" "$WORK/$1/device.crt" "$WORK/secrets-$1/"
    printf '# the operator\n%s\n' "$OPERATOR_PIN" >"$WORK/secrets-$1/$PEERS"
}
store_for guest-a
store_for guest-b
printf '%s' "$PSK_VALUE" >"$WORK/secrets-guest-a/probe.psk"

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "secrets-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

QEMU=
boot() {
    LOG=$WORK/console-$1.log
    echo "adr-030-secrets-proof: booting $1, control plane on $CONTROL_PORT"
    "$HERE/run.sh" "$OUT/rivet-initd" \
        --supervisor "$OUT/rivet-sysd" \
        --cli "$OUT/rivet" \
        --netd "$OUT/rivet-netd" \
        --mcpd "$OUT/rivet-mcpd" \
        --secrets "$WORK/secrets-$1" \
        --document "$DOC" \
        --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
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
}
halt() {
    [ -n "$QEMU" ] || return 0
    kill "$QEMU" 2>/dev/null || true
    wait "$QEMU" 2>/dev/null || true
    QEMU=
}
trap halt EXIT

# `cargo rivet <verb> …` against the guest with `$PIN`, with NO terminal — stdin
# is /dev/null even when this script is run from one. Sets RC.
cr() {
    set +e
    "$CARGO_RIVET" "$@" >"$WORK/out" 2>"$WORK/err" </dev/null
    RC=$?
    set -e
}

# A shell word for each argument, for `script -c`.
quote() {
    for word in "$@"; do
        printf "'%s' " "$(printf '%s' "$word" | sed "s/'/'\\\\''/g")"
    done
}

# `cargo rivet <verb> …` under a REAL pty, typing $1 at every passphrase prompt
# the command shows. Output and errors both land in $WORK/out (a pty has one
# stream) and are copied to $WORK/err so the checks below read either. Sets RC.
typed() {
    TYPE=$1
    shift
    rm -f "$WORK/tty-in"
    mkfifo "$WORK/tty-in"
    : >"$WORK/out"
    COMMAND=$(quote "$CARGO_RIVET" "$@")
    set +e
    timeout 300 script -q -e -c "$COMMAND" /dev/null <"$WORK/tty-in" >"$WORK/out" 2>&1 &
    PTY=$!
    exec 9>"$WORK/tty-in"
    TYPED=0
    while kill -0 "$PTY" 2>/dev/null; do
        PROMPTS=$(grep -o -e 'passphrase for this secrets backup:' -e 'type it again to confirm:' \
            "$WORK/out" 2>/dev/null | wc -l)
        if [ "$PROMPTS" -gt "$TYPED" ]; then
            # The prompt is written a moment before no-echo is switched on.
            sleep 0.5
            printf '%s\n' "$TYPE" >&9
            TYPED=$((TYPED + 1))
        fi
        sleep 0.2
    done
    wait "$PTY"
    RC=$?
    exec 9>&-
    set -e
    cp "$WORK/out" "$WORK/err"
    if grep -qF "$TYPE" "$WORK/out"; then
        fail "the typed passphrase was echoed: $(cat "$WORK/out")"
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
        [ "$ATTEMPT" -ge 6 ] && break
        grep -qiE 'connection (refused|reset)|timed out|os error' "$WORK/err" || break
        ATTEMPT=$((ATTEMPT + 1))
        sleep 2
    done
    [ "$RC" -eq 0 ] || { cat "$WORK/err" >&2; die "\`get status\` exited $RC"; }
}
# The restore plan of $1 against the guest names every one of $2 files
# `identical, skipped` and nothing else: the backup holds the guest's bytes.
plans_identical() {
    # shellcheck disable=SC2046
    typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$1"
    [ "$RC" -eq 0 ] || return 1
    [ "$(grep -c 'identical, skipped' "$WORK/out")" -eq "$2" ] || return 1
    ! grep -qE '  (write|REPLACE)' "$WORK/out"
}
REFUSED_SEAL='wrong passphrase, or this file has been altered'

# ===========================================================================
# Guest A
# ===========================================================================
boot guest-a
PIN=$PIN_A
wait_for_answers

echo
echo "--- backup-secrets: plan, no terminal, then a typed passphrase ---"
# shellcheck disable=SC2046
cr backup-secrets $(endpoint) --into "$WORK/backup"
if [ "$RC" -eq 0 ] && grep -q 'No secret was read' "$WORK/out" && [ ! -e "$WORK/backup" ]; then
    ok "unconfirmed, it is a plan that read no secret and wrote nothing"
else
    fail "the unconfirmed backup was not a plan: $(cat "$WORK/out" "$WORK/err")"
fi
# shellcheck disable=SC2046
cr backup-secrets $(endpoint) --into "$WORK/backup" confirm
if [ "$RC" -ne 0 ] && grep -q 'no terminal' "$WORK/err" && [ ! -e "$WORK/backup" ]; then
    ok "confirmed with no terminal, it is refused and writes nothing"
else
    fail "a backup with no terminal was not refused: $(cat "$WORK/out" "$WORK/err")"
fi
# shellcheck disable=SC2046
typed "$PASSPHRASE" backup-secrets $(endpoint) --into "$WORK/backup" confirm
if [ "$RC" -eq 0 ] && grep -q 'FORGOTTEN PASSPHRASE' "$WORK/out" \
    && [ "$(ls -A "$WORK/backup")" = "$BACKUP_FILE" ]; then
    ok "typed twice, it wrote one file, $BACKUP_FILE, and said what a forgotten passphrase costs"
else
    fail "the typed backup exited $RC: $(cat "$WORK/out"); $(ls -A "$WORK/backup" 2>&1)"
fi
if [ "$(head -c 7 "$WORK/backup/$BACKUP_FILE")" = RVTSECR ] \
    && ! grep -qF "$PSK_VALUE" "$WORK/backup/$BACKUP_FILE" \
    && ! grep -qF 'probe.psk' "$WORK/backup/$BACKUP_FILE" \
    && ! grep -qF "$PIN_A" "$WORK/backup/$BACKUP_FILE"; then
    ok "it is an RVTSECR envelope, with neither probe.psk's value, its name nor the board key readable"
else
    fail "the backup file is not a sealed envelope"
fi
if plans_identical "$WORK/backup" 4; then
    ok "opened with the passphrase, its restore plan names all four files identical to the guest's"
else
    fail "the backup's restore plan was not all identical: $(cat "$WORK/out")"
fi

echo
echo "--- the guest refuses a peers file that locks this operator out ---"
rivet apply "$RESTORE" "$PEERS" --hex "$(printf '%s\n' "$PIN_B" | od -An -tx1 | tr -d ' \n')" confirm
if [ "$RC" -ne 0 ] && grep -q 'E_UNSAFE' "$WORK/err" && grep -q "$OPERATOR_PIN" "$WORK/err"; then
    ok "refused, E_UNSAFE, naming the session's key"
else
    fail "a lock-out peers file was not refused: $(cat "$WORK/out" "$WORK/err")"
fi

echo
echo "--- damage the guest's store, then restore ---"
rivet apply remove-file "$STORE/probe.psk" confirm
[ "$RC" -eq 0 ] && ok "probe.psk removed on the guest" || fail "could not remove probe.psk: $(cat "$WORK/err")"
rivet apply "$RESTORE" device.crt --hex 00 confirm
[ "$RC" -eq 0 ] && ok "device.crt overwritten on the guest" || fail "could not overwrite device.crt: $(cat "$WORK/err")"

# shellcheck disable=SC2046
typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$WORK/backup" confirm
if [ "$RC" -ne 0 ] && grep -q -- '--replace' "$WORK/err"; then
    ok "without --replace, a differing device.crt refuses the restore"
else
    fail "the restore without --replace was not refused: $(cat "$WORK/out")"
fi
rivet get file "$STORE/probe.psk"
if [ "$RC" -eq 0 ] && grep -q '"present":false' "$WORK/out"; then
    ok "and nothing was written — probe.psk is still absent"
else
    fail "the refused restore wrote something: $(cat "$WORK/out" "$WORK/err")"
fi

# shellcheck disable=SC2046
typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$WORK/backup" --replace
if [ "$RC" -eq 0 ] && grep -q 'probe.psk' "$WORK/out" && grep -q 'REPLACE' "$WORK/out"; then
    ok "the plan names probe.psk to write and device.crt to replace"
else
    fail "the restore plan was wrong: $(cat "$WORK/out")"
fi
# shellcheck disable=SC2046
typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$WORK/backup" --replace confirm
if [ "$RC" -eq 0 ] && grep -q 'read back identical' "$WORK/out"; then
    ok "restored, each write read back identical"
else
    fail "the confirmed restore exited $RC: $(cat "$WORK/out")"
fi
if plans_identical "$WORK/backup" 4; then
    ok "a restore plan after the restore names all four files identical — the store is the original"
else
    fail "the store after the restore is not the original: $(cat "$WORK/out")"
fi

echo
echo "--- a wrong passphrase and a tampered backup are refused alike ---"
# shellcheck disable=SC2046
typed "$WRONG" restore-secrets $(endpoint) --from "$WORK/backup" --replace confirm
if [ "$RC" -ne 0 ] && grep -qF "$REFUSED_SEAL" "$WORK/out" && grep -q 'NOTHING WAS SENT' "$WORK/out"; then
    ok "a wrong passphrase: refused, nothing sent"
else
    fail "a wrong passphrase was not refused: $(cat "$WORK/out")"
fi
cp -a "$WORK/backup" "$WORK/tampered"
SIZE=$(wc -c <"$WORK/tampered/$BACKUP_FILE")
AT=$((SIZE - 24))
# A byte that is certainly different from the one there.
if [ "$(od -An -tx1 -j "$AT" -N1 "$WORK/tampered/$BACKUP_FILE" | tr -d ' \n')" = 58 ]; then
    NEW=Y
else
    NEW=X
fi
printf '%s' "$NEW" | dd of="$WORK/tampered/$BACKUP_FILE" bs=1 seek="$AT" conv=notrunc 2>/dev/null
# shellcheck disable=SC2046
typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$WORK/tampered" --replace confirm
if [ "$RC" -ne 0 ] && grep -qF "$REFUSED_SEAL" "$WORK/out" && grep -q 'NOTHING WAS SENT' "$WORK/out"; then
    ok "one byte changed in the envelope, the right passphrase: the same refusal, nothing sent"
else
    fail "the tampered backup was not refused: $(cat "$WORK/out")"
fi

echo
echo "--- the unencrypted layout an earlier build wrote is refused ---"
mkdir -p "$WORK/unencrypted/files"
cp "$WORK/secrets-guest-a/probe.psk" "$WORK/unencrypted/files/"
printf 'format = rivet-machine-secrets 1\nboard-key = %s\n' "$PIN_A" >"$WORK/unencrypted/$UNENCRYPTED_MANIFEST"
# shellcheck disable=SC2046
cr restore-secrets $(endpoint) --from "$WORK/unencrypted" --replace confirm
if [ "$RC" -ne 0 ] && grep -q 'UNENCRYPTED' "$WORK/err" && grep -q 'NOTHING WAS SENT' "$WORK/err" \
    && ! grep -q 'terminal' "$WORK/err"; then
    ok "refused by name before any passphrase was asked for"
else
    fail "the unencrypted layout was not refused: $(cat "$WORK/out" "$WORK/err")"
fi

halt

# ===========================================================================
# Guest B — another identity
# ===========================================================================
echo
boot guest-b
PIN=$PIN_B
wait_for_answers

echo "--- guest A's backup onto guest B ---"
# shellcheck disable=SC2046
typed "$PASSPHRASE" backup-secrets $(endpoint) --into "$WORK/guest-b-before" confirm
[ "$RC" -eq 0 ] && ok "guest B backed up first, for comparison" \
    || fail "could not back up guest B: $(cat "$WORK/out")"
# shellcheck disable=SC2046
typed "$PASSPHRASE" restore-secrets $(endpoint) --from "$WORK/backup" --replace confirm
if [ "$RC" -ne 0 ] && grep -q 'DIFFERENT BOARD' "$WORK/out" && grep -q "$PIN_A" "$WORK/out" \
    && grep -q 'NOTHING WAS SENT' "$WORK/out"; then
    ok "refused as a DIFFERENT BOARD once opened, naming guest A's key, nothing sent"
else
    fail "guest A's backup was not refused on guest B: $(cat "$WORK/out")"
fi
if plans_identical "$WORK/guest-b-before" 3; then
    ok "guest B's store is unchanged — its own earlier backup still plans all identical"
else
    fail "guest B's store changed: $(cat "$WORK/out")"
fi

halt
echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-030-secrets-proof: PASS — sealed under a typed passphrase, restored onto the same guest, refused on another"
else
    echo "adr-030-secrets-proof: FAIL — the console logs are in $WORK" >&2
fi
exit "$VERDICT"
