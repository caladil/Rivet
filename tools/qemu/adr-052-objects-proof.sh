#!/bin/sh
# tools/qemu/adr-052-objects-proof.sh — **a fleet server's application objects,
# sent with `cargo rivet fleet sync` into its repository store on a booted
# guest, verified at the door, listed back, and pruned by the manifest.**
#
# `ADR-052` §7 and §9 step 4, `.rulings/` R-182 and R-184 D1/D2. Run it in the
# container:
#
#   tools/container/exec.sh sh tools/qemu/adr-052-objects-proof.sh
#
# ## What it asserts
#
# One guest, booted with the real control plane, the installer, the vendored
# `git` and `git-http-backend`, a trust store holding ONE throwaway key minted
# by this run, and a document declaring `[repos.fleet]`. The real `rivet-fleetd`,
# signed with that key, is installed over the control plane so its data
# directory exists — it is not enabled and serves nothing here.
#
# 1. **`fleet push` puts the text**, and **`fleet sync` without `confirm` sends
#    nothing** and names the two objects it would send.
# 2. **`fleet sync confirm` sends two real signed objects** — `hello-rivet` at
#    two versions, packaged by `cargo rivet package` with the throwaway key —
#    and the store holds exactly those two digests, read off the machine with
#    `get file`, never from what the client printed.
# 3. **A rerun sends nothing.**
# 4. **A tampered object is refused**: a third version whose bytes were altered
#    after it was published is refused at the door with the transfer intact,
#    and is not held.
# 5. **A system update is held** (`ADR-063` §6): a signed system update,
#    published into a scratch repository, is sent, admitted by its signed
#    header's kind, and held beside the two applications.
# 6. **A caller without the operator's key is refused**: a sync presenting an
#    identity the guest's `peers` does not name asks for nothing and sends
#    nothing, and the store is unchanged.
# 7. **Prune removes exactly the object the manifest stops naming**: with one
#    entry removed and committed, `--prune` is not run while the server's
#    `main` is behind; after `fleet push`, `sync --prune confirm` removes that
#    digest and the update this manifest never named, and keeps what it names.
#
# ## What it does NOT settle
#
# `ADR-026` §4: no Raspberry Pi, no SD card, and QEMU's SLIRP forwarding is not
# `rivet-netd`'s path. `/state` is the initramfs tmpfs, not ext4 on a card. The
# objects are a few hundred kilobytes each, sent over loopback; a realistic
# fleet's sizes over a real network, and the memory a board spends on them, are
# not measured. Members fetching from the store are not designed (`ADR-052` §9
# step 5) and not exercised. Every identity and key is THROWAWAY, minted into
# this run's working directory.
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
CROSS_CC="$BR/host/bin/aarch64-linux-gcc"
CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
export CC_aarch64_unknown_linux_musl

fail() { echo; echo "adr-052-objects-proof: FAIL: $*" >&2; exit 1; }
note() { echo "adr-052-objects-proof: $*"; }

command -v qemu-system-aarch64 >/dev/null 2>&1 \
    || fail "qemu-system-aarch64 is not on PATH. tools/container/exec.sh puts it there."
command -v timeout >/dev/null 2>&1 || fail "coreutils 'timeout' is not on PATH"
command -v git >/dev/null 2>&1 || fail "no host git on PATH: the fleet repository is git"
command -v openssl >/dev/null 2>&1 || fail "no openssl on PATH: the throwaway key and ids are minted with it"
[ -x "$CC_aarch64_unknown_linux_musl" ] \
    || fail "no cross C compiler at $CC_aarch64_unknown_linux_musl — rivetos-mcpd links ring"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi
[ -n "${RIVET_KERNEL:-}" ] || fail "no RIVET_KERNEL and no /br/images/Image (--net needs ADR-024's kernel)"

GUEST_GIT=$BR/target/usr/bin/git
GUEST_HTTP_BACKEND=$BR/target/usr/libexec/git-core/git-http-backend
[ -f "$GUEST_GIT" ] || fail "no vendored git at $GUEST_GIT"
[ -f "$GUEST_HTTP_BACKEND" ] || fail "no git-http-backend at $GUEST_HTTP_BACKEND"

constant() {
    _value=$(sed -n "s/^pub const $2: *[a-z0-9&]* *= *\"\{0,1\}\([^\";]*\)\"\{0,1\};.*/\1/p" "$1" | head -1)
    [ -n "$_value" ] || fail "cannot read $2 out of $1 — this harness agrees with that crate and will not guess"
    printf '%s\n' "$_value"
}
CONTROL_PORT=$(constant "$REPO/rivet-config/src/control_plane.rs" DEFAULT_PORT)
SERVED=$(constant "$REPO/rivet-fleet-repo/src/lib.rs" SERVED_NAME)
MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
FLEETD=$(constant "$REPO/rivet-core/src/repository_objects.rs" APPLICATION)
STORE_DIR=$(constant "$REPO/rivet-core/src/repository_objects.rs" DIR)
BUILT_IN=$(constant "$REPO/rivet-core/src/service.rs" BUILT_IN_VOLUME)
DATA_DIR=$BUILT_IN/app/$FLEETD/data
OBJECTS=$DATA_DIR/$STORE_DIR/objects

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr052-objects.XXXXXX") || fail "no working directory"
chmod 700 "$WORK"
QEMU_PID=
cleanup() {
    [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# **The vendored git may be dynamic** (run.sh's --git says why), so the
# libraries it NEEDs are staged beside the loader run.sh's --libraries packs,
# found by the cross readelf in the Buildroot tree that built them — the same
# staging `adr-061-gantry-proof.sh` does.
READELF=$BR/host/bin/aarch64-linux-readelf
[ -x "$READELF" ] || fail "no cross readelf at $READELF"
mkdir -p "$WORK/libs"
stage_needed() {
    for _lib in $("$READELF" -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p'); do
        [ "$_lib" = libc.so ] && continue
        [ -e "$WORK/libs/$_lib" ] && continue
        _found=
        for _dir in "$BR/target/usr/lib" "$BR/target/lib"; do
            [ -e "$_dir/$_lib" ] && { _found=$_dir/$_lib; break; }
        done
        [ -n "$_found" ] || fail "$1 NEEDs $_lib and no library directory under $BR/target holds it"
        cp -L "$_found" "$WORK/libs/$_lib"
        stage_needed "$WORK/libs/$_lib"
    done
}
stage_needed "$GUEST_GIT"
stage_needed "$GUEST_HTTP_BACKEND"

# **Two sorted lists, compared without git.** `git diff --no-index` still looks
# for a repository from the working directory, and in a linked worktree mounted
# into the container `.git` is a file naming a host path, so git refuses before
# it compares anything. The build container has no `cmp` either, so the shell
# compares the two texts itself.
same_lines() {
    [ "$(cat "$1")" = "$(cat "$2")" ] && return 0
    echo "expected ($1):"
    cat "$1"
    echo "held ($2):"
    cat "$2"
    return 1
}

# **Nothing here may read this host's git configuration, credentials or ~/.rivet.**
HOME=$WORK/home
mkdir -p "$HOME"
export HOME
GIT_CONFIG_NOSYSTEM=1
GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_NOSYSTEM GIT_TERMINAL_PROMPT
unset RIVET_APP_SEED RIVET_SYSTEM_SEED RIVET_FLEET_REPOSITORY 2>/dev/null || true

# --- 1. the binaries --------------------------------------------------------

note "building the guest's binaries for $TARGET"
cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd -p rivetos-deploy \
    || fail "the guest build failed — see above"
note "building cargo-rivet and the fixture signer for the host"
cargo build --release -p cargo-rivet || fail "cargo build -p cargo-rivet failed"
CARGO_RIVET=$HOST_OUT/cargo-rivet
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"
cargo build --release --manifest-path "$HERE/fixture-signer/Cargo.toml" \
    || fail "the fixture signer would not build"
FIXTURE_SIGNER=$HOST_OUT/rivet-fixture-signer
[ -x "$FIXTURE_SIGNER" ] || fail "no rivet-fixture-signer at $FIXTURE_SIGNER"

# --- 2. a THROWAWAY signing key, and the artifacts it signs -----------------
#
# Thirty-two bytes from the OS, in this run's working directory and nowhere
# else. The guest trusts its public half for applications and updates, so a
# refusal below is never about an unknown key unless it is meant to be.
KEY=$WORK/throwaway-app.hex
openssl rand -hex 32 >"$KEY" || fail "could not mint a throwaway key"
PUB=$WORK/throwaway-app.pub

stage_crate() {
    mkdir -p "$2"
    cp -a "$1/Cargo.toml" "$1/src" "$2/" || fail "could not stage $1 at $2"
    sed -i "0,/^version = /s/^version = .*/version = \"$3\"/" "$2/Cargo.toml"
    grep -q "^version = \"$3\"\$" "$2/Cargo.toml" || fail "the version in $2/Cargo.toml is not $3"
}
package() {
    _crate=$1
    _log=$2
    shift 2
    "$CARGO_RIVET" package --manifest-path "$_crate/Cargo.toml" --target "$TARGET" \
        --out-dir "$WORK/rvt-$(basename "$_crate")" --no-assets \
        --signing-key "$KEY" --public-key-out "$PUB" "$@" >"$_log" 2>&1 \
        || { cat "$_log" >&2; fail "cargo rivet package failed for $_crate"; }
}
rvt_in() {
    _found=$(find "$1" -name '*.rvt' -type f | head -1)
    [ -n "$_found" ] || fail "no .rvt under $1"
    printf '%s\n' "$_found"
}

HELLO_SRC=$REPO/examples/probes/hello-rivet
[ -d "$HELLO_SRC" ] || fail "no crate at $HELLO_SRC"
note "packaging hello-rivet 0.1.1, 0.1.2 and 0.1.3 with the throwaway key"
for _v in 0.1.1 0.1.2 0.1.3; do
    stage_crate "$HELLO_SRC" "$WORK/hello-$_v" "$_v"
    package "$WORK/hello-$_v" "$WORK/package-hello-$_v.log" --target-dir "$WORK/hello-$_v/target"
done
RVT1=$(rvt_in "$WORK/rvt-hello-0.1.1")
RVT2=$(rvt_in "$WORK/rvt-hello-0.1.2")
RVT3=$(rvt_in "$WORK/rvt-hello-0.1.3")

note "packaging the real rivet-fleetd with the throwaway key, so the guest has its data directory"
# **Stripped first**: the release build carries debug symbols past the
# supervisor's launch-hash ceiling, and `cargo rivet package` refuses it by
# name. The cross toolchain's strip, as that refusal says.
cargo build --release --target "$TARGET" -p rivetos-fleetd || fail "the rivet-fleetd build failed"
STRIP=${RIVET_STRIP:-$BR/host/bin/aarch64-linux-strip}
[ -x "$STRIP" ] || fail "no cross strip at $STRIP"
cp "$OUT/rivet-fleetd" "$WORK/rivet-fleetd" && "$STRIP" "$WORK/rivet-fleetd" || fail "could not strip rivet-fleetd"
package "$REPO/cmd/rivet-fleetd" "$WORK/package-fleetd.log" --target-dir "$TARGET_DIR" \
    --binary "$WORK/rivet-fleetd"
RVT_FLEETD=$(rvt_in "$WORK/rvt-rivet-fleetd")

note "wrapping a throwaway system update, which the store holds (ADR-063 6)"
mkdir -p "$WORK/update/a" "$WORK/update/b" "$WORK/update/out"
printf 'not a system image\n' >"$WORK/update/system.img"
printf 'config\n' >"$WORK/update/a/config.txt"
printf 'config\n' >"$WORK/update/b/config.txt"
"$CARGO_RIVET" update --system "$WORK/update/system.img" --boot-a "$WORK/update/a" \
    --boot-b "$WORK/update/b" --signing-key "$KEY" --version 0.0.1 \
    --out-dir "$WORK/update/out" >"$WORK/update.log" 2>&1 \
    || { cat "$WORK/update.log" >&2; fail "cargo rivet update failed"; }
RVT_UPDATE=$(rvt_in "$WORK/update/out")

TRUST=$WORK/trust
"$FIXTURE_SIGNER" trust-store "$TRUST" "throwaway:application,update:$PUB" \
    || fail "could not write the trust store"

# --- 3. identities, peers, and the document ---------------------------------

GUEST_NAME=rivet-adr052o-guest
mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$2" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; fail "could not mint the $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
GUEST_PIN=$(mint secrets "$GUEST_NAME")
OPERATOR_PIN=$(mint operator rivet-adr052o-operator)
STRANGER_PIN=$(mint stranger rivet-adr052o-stranger)
[ -n "$GUEST_PIN" ] && [ -n "$OPERATOR_PIN" ] && [ -n "$STRANGER_PIN" ] || fail "an identity has no readable pin"
[ "$OPERATOR_PIN" != "$STRANGER_PIN" ] || fail "the operator and the stranger minted one key"
printf '# the operator this run minted, and nobody else\n%s\n' "$OPERATOR_PIN" \
    >"$WORK/secrets/${MCPD_PEERS_PATH##*/}" || fail "could not write peers"

REPO_ID=$(openssl rand -hex 8)
DOC=$WORK/machine.toml
{
    printf 'version = 1\nhostname = "adr052o-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
    printf '\n[repos.%s]\nid = "%s"\n' "$SERVED" "$REPO_ID"
} >"$DOC" || fail "could not compose $DOC"

# --- 4. the machine ---------------------------------------------------------

CONSOLE=${RIVET_PROOF_LOG:-$REPO/out/adr-052-objects-proof.console.log}
mkdir -p "$(dirname "$CONSOLE")"
: >"$CONSOLE"
mark() { wc -l <"$CONSOLE" | tr -d ' '; }
since() { tail -n "+$(($1 + 1))" "$CONSOLE"; }

note "booting with the installer, git, the throwaway trust store and [repos.$SERVED]"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --deploy "$OUT/rivet-deploy" \
    --git "$GUEST_GIT" \
    --git-http-backend "$GUEST_HTTP_BACKEND" \
    --libraries "$WORK/libs" \
    --secrets "$WORK/secrets" \
    --trust "$TRUST" \
    --document "$DOC" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
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

# **The digests the store holds, read off the machine.** `get file` on the
# objects directory lists one entry per digest.
held_digests() {
    ask get file "$OBJECTS"
    if [ "$ASK_RC" -ne 0 ]; then
        : >"$WORK/held"
        return 0
    fi
    grep -oE '[0-9a-f]{64}' "$WORK/ask-out" | sort -u >"$WORK/held"
}

# --- 5. rivet-fleetd installed, so its data directory exists ----------------

note "installing $(basename "$RVT_FLEETD") over the control plane (not enabled)"
MARK_INSTALL=$(mark)
set +e
# `install`, the scripted verb: `deploy` is the interview for a person at a
# terminal and refuses arguments. `--no-wait`, because the fleetd install's
# data directory is what the loop below waits for.
env RIVET_APP_SEED="$KEY" "$CARGO_RIVET" install --host "127.0.0.1:$CONTROL_PORT" \
    --identity "$WORK/operator" --pin "$GUEST_PIN" --at "$WORK/deploy-fleet" \
    --rvt "$RVT_FLEETD" --no-wait confirm >"$WORK/deploy-fleetd.log" 2>&1
DEPLOY_RC=$?
set -e
cat "$WORK/deploy-fleetd.log"
[ "$DEPLOY_RC" -eq 0 ] || fail "deploying rivet-fleetd exited $DEPLOY_RC"
ATTEMPT=1
while :; do
    ask get file "$DATA_DIR"
    [ "$ASK_RC" -eq 0 ] && grep -qE '"present": ?true' "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 120 ] && { cat "$WORK/ask-out" "$WORK/ask-err" >&2; since "$MARK_INSTALL" | grep -a 'rivet-deploy\|data directory' | tail -20 >&2
        fail "$DATA_DIR never appeared after rivet-fleetd was installed"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done
note "$DATA_DIR exists"

# --- Q1: the text, and a plan that sends nothing ----------------------------

FLEET=$WORK/fleet
"$CARGO_RIVET" fleet init --at "$FLEET" >"$WORK/init.log" 2>&1 || { cat "$WORK/init.log" >&2; fail "fleet init"; }
publish() {
    "$CARGO_RIVET" fleet publish "$2" --trusted-key "$PUB" --at "$1" >"$WORK/publish.log" 2>&1 \
        || { cat "$WORK/publish.log" >&2; fail "fleet publish $2 into $1"; }
    grep -oE '[0-9a-f]{64}' "$WORK/publish.log" | head -1
}
DIG1=$(publish "$FLEET" "$RVT1")
DIG2=$(publish "$FLEET" "$RVT2")
[ -n "$DIG1" ] && [ -n "$DIG2" ] && [ "$DIG1" != "$DIG2" ] || fail "the two published digests are not two digests"
[ -f "$FLEET/objects/$DIG1" ] && [ -f "$FLEET/objects/$DIG2" ] || fail "fleet publish left no objects"

run_fleet() {
    _what=$1
    _log=$2
    shift 2
    set +e
    "$CARGO_RIVET" fleet "$_what" --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/${WHO:-operator}" \
        --pin "$GUEST_PIN" "$@" >"$_log" 2>&1
    FLEET_RC=$?
    set -e
    echo "--- cargo rivet fleet $_what $* (as ${WHO:-operator}) exited $FLEET_RC ---"
    cat "$_log"
}

run_fleet push "$WORK/push1.log" --at "$FLEET" confirm
[ "$FLEET_RC" -eq 0 ] && grep -qF PUSHED "$WORK/push1.log" || fail "fleet push did not push"

run_fleet sync "$WORK/plan.log" --at "$FLEET"
[ "$FLEET_RC" -eq 0 ] || fail "the sync plan exited $FLEET_RC"
grep -qF "would send $DIG1" "$WORK/plan.log" && grep -qF "would send $DIG2" "$WORK/plan.log" \
    || fail "the plan did not name both objects"
grep -qF "Nothing was sent" "$WORK/plan.log" || fail "the plan did not say nothing was sent"
held_digests
[ ! -s "$WORK/held" ] || { cat "$WORK/held" >&2; fail "the store holds objects after a plan"; }
note "Q1: the text is pushed; a sync plan named both objects and the store is empty"

# --- Q2: two real signed objects, sent and listed ----------------------------

MARK_SYNC=$(mark)
run_fleet sync "$WORK/sync.log" --at "$FLEET" confirm
[ "$FLEET_RC" -eq 0 ] || fail "fleet sync confirm exited $FLEET_RC"
grep -qF "sent      $DIG1" "$WORK/sync.log" && grep -qF "sent      $DIG2" "$WORK/sync.log" \
    || fail "the sync did not report sending both objects"
grep -qF "SYNCED" "$WORK/sync.log" || fail "the sync did not report SYNCED"
held_digests
echo "--- get file $OBJECTS, off the machine ---"
cat "$WORK/ask-out"
printf '%s\n%s\n' "$DIG1" "$DIG2" | sort -u >"$WORK/expected"
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "the store does not hold exactly the two objects sent"; }
since "$MARK_SYNC" | grep -aq 'REFUSED AT THE DOOR' && fail "the machine refused something during a clean sync"
# **Only the object files are handed to rivet-fleetd.** The store's directories
# stay root's, mode 0750, with the group its data directory carries as that
# application's read grant; each object file carries the data directory's uid,
# which is that application's identity. An application that owned a directory
# root writes into could plant a link in it.
field() {
    grep -oE "\"$1\": ?\"?[0-9]+" "$WORK/ask-out" | head -1 | sed 's/.*[^0-9]//'
}
OBJECTS_UID=$(field uid)
OBJECTS_GID=$(field gid)
OBJECTS_MODE=$(field mode)
ask get file "$OBJECTS/$DIG1"
DIGEST_UID=$(field uid)
DIGEST_GID=$(field gid)
DIGEST_MODE=$(field mode)
OBJECT_NAME=$(grep -oE '[A-Za-z0-9._+-]+\.rvt' "$WORK/ask-out" | head -1)
[ -n "$OBJECT_NAME" ] || { cat "$WORK/ask-out" >&2; fail "get file $OBJECTS/$DIG1 names no object file"; }
ask get file "$OBJECTS/$DIG1/$OBJECT_NAME"
FILE_UID=$(field uid)
ask get file "$DATA_DIR"
DATA_UID=$(field uid)
DATA_GID=$(field gid)
[ -n "$DATA_UID" ] && [ -n "$DATA_GID" ] || fail "$DATA_DIR answered no owner"
[ "$OBJECTS_UID" = 0 ] && [ "$DIGEST_UID" = 0 ] \
    || fail "the store's directories are owned ($OBJECTS_UID, $DIGEST_UID), not by root"
[ "$OBJECTS_GID" = "$DATA_GID" ] && [ "$DIGEST_GID" = "$DATA_GID" ] \
    || fail "the store's directories' group ($OBJECTS_GID, $DIGEST_GID) is not $DATA_DIR's ($DATA_GID)"
[ "$OBJECTS_MODE" = 0750 ] && [ "$DIGEST_MODE" = 0750 ] \
    || fail "the store's directories are mode ($OBJECTS_MODE, $DIGEST_MODE), not 0750"
[ "$FILE_UID" = "$DATA_UID" ] \
    || fail "$OBJECT_NAME is owned $FILE_UID, not as $DATA_DIR is ($DATA_UID)"
note "the store's directories are root's, mode 0750, group $DATA_GID; its object files are owned $DATA_UID, as $FLEETD's data directory is"
note "Q2: fleet sync sent $DIG1 and $DIG2; the machine's store holds exactly those"

# --- Q3: a rerun sends nothing ----------------------------------------------

run_fleet sync "$WORK/rerun.log" --at "$FLEET" confirm
[ "$FLEET_RC" -eq 0 ] || fail "the rerun exited $FLEET_RC"
grep -qF "0 sent, 2 already held" "$WORK/rerun.log" || fail "the rerun did not report 0 sent, 2 already held"
if grep -q '^  sent ' "$WORK/rerun.log"; then fail "the rerun sent something"; fi
note "Q3: a rerun sent nothing"

# --- Q4: a tampered object is refused ----------------------------------------

BAD=$WORK/fleet-tampered
cp -a "$FLEET" "$BAD"
DIG3=$(publish "$BAD" "$RVT3")
# The last byte of the program member, changed after the artifact was
# published: the header's signature still verifies, a member does not.
_size=$(wc -c <"$BAD/objects/$DIG3" | tr -d ' ')
printf '\377' | dd of="$BAD/objects/$DIG3" bs=1 seek=$((_size - 1)) conv=notrunc 2>/dev/null \
    || fail "could not alter the object"
MARK_BAD=$(mark)
run_fleet sync "$WORK/tampered.log" --at "$BAD" confirm
[ "$FLEET_RC" -ne 0 ] || fail "a sync carrying a tampered object exited 0"
grep -qF "refused   $DIG3" "$WORK/tampered.log" || fail "the tampered object was not reported refused"
grep -qF "REFUSED THE ARTIFACT" "$WORK/tampered.log" || fail "the refusal is not the ARTIFACT's"
grep -qF "not a transfer failure" "$WORK/tampered.log" || fail "the refusal did not say the transfer was intact"
held_digests
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "the tampered object changed what the store holds"; }
since "$MARK_BAD" | grep -aq 'REFUSED AT THE DOOR of the repository store' \
    || fail "rivet-mcpd did not record refusing the tampered object"
note "Q4: the tampered object was refused at the door and not held"

# --- Q5: a system update is held -----------------------------------------------
#
# ADR-063 §6: a fleet server's store holds every kind, system updates and cards
# included. The guest trusts the throwaway key for updates, so the update is
# admitted by its signed header's kind and held beside the two applications.

KIND=$WORK/fleet-kind
"$CARGO_RIVET" fleet init --at "$KIND" >"$WORK/init-kind.log" 2>&1 || { cat "$WORK/init-kind.log" >&2; fail "fleet init (kind)"; }
DIG_UPDATE=$(publish "$KIND" "$RVT_UPDATE")
grep -q 'kind    = "update"' "$KIND/manifest.toml" || fail "the update was not recorded as an update"
run_fleet sync "$WORK/kind.log" --at "$KIND" confirm
[ "$FLEET_RC" -eq 0 ] || fail "a sync carrying a trusted update exited $FLEET_RC"
grep -qF "sent      $DIG_UPDATE" "$WORK/kind.log" || fail "the update was not reported sent"
held_digests
printf '%s\n%s\n%s\n' "$DIG1" "$DIG2" "$DIG_UPDATE" | sort -u >"$WORK/expected"
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "the store does not hold the update beside the two applications"; }
note "Q5: a signed system update was admitted by kind and held"

# --- Q6: a caller without the operator's key ---------------------------------

WHO=stranger
run_fleet sync "$WORK/stranger.log" --at "$BAD" confirm
WHO=operator
[ "$FLEET_RC" -ne 0 ] || fail "a sync by an identity not in peers exited 0"
grep -qF "nothing was sent" "$WORK/stranger.log" || fail "the stranger's sync did not say nothing was sent"
held_digests
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "a refused caller changed what the store holds"; }
note "Q6: an identity not in peers was refused and the store is unchanged"

# --- Q7: prune exactly what the manifest stops naming ------------------------

awk -v d="$DIG1" '
    /^\[\[artifact\]\]$/ { if (inblock && keep) printf "%s", buf; buf = $0 "\n"; keep = 1; inblock = 1; next }
    inblock { buf = buf $0 "\n"; if (index($0, d)) keep = 0; next }
    { print }
    END { if (inblock && keep) printf "%s", buf }
' "$FLEET/manifest.toml" >"$WORK/manifest.new" || fail "could not rewrite the manifest"
cp "$WORK/manifest.new" "$FLEET/manifest.toml"
grep -qF "$DIG1" "$FLEET/manifest.toml" && fail "the manifest still names $DIG1"
grep -qF "$DIG2" "$FLEET/manifest.toml" || fail "the rewrite dropped $DIG2 too"
git -C "$FLEET" -c user.name=proof -c user.email=proof@rivet.invalid commit -q -am "stop naming hello-rivet 0.1.1"

run_fleet sync "$WORK/prune-behind.log" --at "$FLEET" --prune confirm
[ "$FLEET_RC" -ne 0 ] || fail "a prune against a server whose main is behind exited 0"
grep -qF "PRUNE NOT RUN" "$WORK/prune-behind.log" || fail "the prune was not held back while the server's main is behind"
held_digests
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "something was removed while the server's main was behind"; }

run_fleet push "$WORK/push2.log" --at "$FLEET" confirm
[ "$FLEET_RC" -eq 0 ] && grep -qF PUSHED "$WORK/push2.log" || fail "the second push did not push"
run_fleet sync "$WORK/prune-plan.log" --at "$FLEET" --prune
grep -qF "would prune $DIG1" "$WORK/prune-plan.log" || fail "the prune plan did not name $DIG1"
held_digests
same_lines "$WORK/expected" "$WORK/held" >"$WORK/diff" 2>&1 \
    || fail "a prune plan removed something"

run_fleet sync "$WORK/prune.log" --at "$FLEET" --prune confirm
[ "$FLEET_RC" -eq 0 ] || fail "sync --prune confirm exited $FLEET_RC"
grep -qF "pruned    $DIG1" "$WORK/prune.log" || fail "the prune did not report removing $DIG1"
if grep -qF "pruned    $DIG2" "$WORK/prune.log"; then fail "the prune removed $DIG2, which the manifest names"; fi
# The update came from another repository, so this manifest never named it.
grep -qF "pruned    $DIG_UPDATE" "$WORK/prune.log" || fail "the prune kept $DIG_UPDATE, which this manifest does not name"
held_digests
echo "--- get file $OBJECTS after the prune ---"
cat "$WORK/ask-out"
printf '%s\n' "$DIG2" >"$WORK/expected-after"
same_lines "$WORK/expected-after" "$WORK/held" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "after the prune the store does not hold exactly $DIG2"; }
note "Q7: with 0.1.1 dropped and pushed, sync --prune removed $DIG1 and the unnamed update, and kept $DIG2"

echo
echo "adr-052-objects-proof: PASS"
echo
echo "  What ran, on $RIVET_KERNEL under QEMU -M virt:"
echo "    * the real rivet-fleetd, signed with a throwaway key, installed so its data"
echo "      directory exists; fleet push put the fleet repository's text in place;"
echo "    * fleet sync planned (nothing sent), then sent two signed hello-rivet"
echo "      objects; get file listed exactly those two digests in the store;"
echo "    * a rerun sent nothing;"
echo "    * a tampered object was refused at the door and not held; a signed system"
echo "      update was admitted and held (ADR-063 6); an identity not in peers was"
echo "      refused and changed nothing;"
echo "    * --prune was held back while the server's main was behind, and after a"
echo "      push removed exactly what the manifest does not name."
echo
echo "  What it does NOT settle: a board, a card's ext4 /state, rivet-netd's own"
echo "  network path, realistic object sizes over a real network, or members"
echo "  fetching from the store. The console is $CONSOLE."
