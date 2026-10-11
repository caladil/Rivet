#!/bin/sh
# tools/qemu/adr-035-git-proof.sh — **a fleet server's `fleet` repository,
# created by the supervisor from its document, pushed to with `cargo rivet
# fleet push`, and read back over the control plane by a second, independent
# git client.**
#
# `ADR-035` §4 and `ADR-052` §9 step 4, `.rulings/` R-182 and R-184. Run it in
# the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-035-git-proof.sh
#
# ## What it asserts
#
# One guest, booted with the real control plane, the vendored `git` and
# `git-http-backend` packed where a card puts them, and a document declaring
# `[repos.fleet]` with a minted identifier — exactly the table
# `cmd/rivet-provision` now writes into every fleet server's document:
#
# 1. **`rivet-sysd` created the bare repository** — `/state/git/fleet.git`,
#    read off the machine with `get file`, its `HEAD` naming `main`.
# 2. **`cargo rivet fleet push` without `confirm` pushes nothing**, and says it
#    would create `main`; **with `confirm` it pushes** a scratch fleet
#    repository holding real commits, and reads the server's `main` back.
# 3. **A second client clones it back over the same control plane** — the
#    container's own stock `git`, over TLS it verifies itself: the guest's
#    certificate as its only trust anchor, its public key pinned with
#    `http.pinnedPubkey`, the operator's certificate and key presented, and the
#    guest's minted name resolved to the forwarded port. Refs, tree and every
#    file's bytes are compared with the scratch repository.
# 4. **A second push of a new commit fast-forwards**, and a `git fetch` in the
#    clone — whose negotiation is made larger than 1 KiB, so git gzips the
#    request body — brings it back.
# 5. **A caller whose key is not in `peers` is refused**: `cargo rivet fleet
#    push` with a third identity fails naming the control plane, and so does a
#    stock `git ls-remote` presenting that identity. The server's `main` is
#    unchanged.
# 6. **Uncommitted work and a push that is not a fast-forward are refused**
#    against the real server, and neither changes its `main`.
#
# ## What it does NOT settle
#
# `ADR-026` §4: no Raspberry Pi, no SD card, and QEMU's SLIRP forwarding is not
# `rivet-netd`'s path. `/state` is the initramfs tmpfs, not ext4 on a card —
# the repository dies with the guest. The fleet repository pushed is a few
# kilobytes; a realistic one's size, and the memory the machine spends holding
# a git answer whole, are not measured here. Application OBJECTS are not part
# of this at all: they are not in git (`ADR-052` §3), R-182 keeps them off the
# card, and `tools/qemu/adr-052-objects-proof.sh` proves the route that sends
# them, `cargo rivet fleet sync`. Every identity is THROWAWAY,
# minted into this run's working directory.
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

fail() { echo; echo "adr-035-git-proof: FAIL: $*" >&2; exit 1; }
note() { echo "adr-035-git-proof: $*"; }

command -v qemu-system-aarch64 >/dev/null 2>&1 \
    || fail "qemu-system-aarch64 is not on PATH. tools/container/exec.sh puts it there."
command -v timeout >/dev/null 2>&1 || fail "coreutils 'timeout' is not on PATH"
command -v git >/dev/null 2>&1 || fail "no host git on PATH: both the pusher and the second client are git"
command -v openssl >/dev/null 2>&1 \
    || fail "no openssl on PATH: the second client's key and pin are derived with it"
[ -x "$CC_aarch64_unknown_linux_musl" ] \
    || fail "no cross C compiler at $CC_aarch64_unknown_linux_musl — rivetos-mcpd links ring"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi
[ -n "${RIVET_KERNEL:-}" ] || fail "no RIVET_KERNEL and no /br/images/Image (--net needs ADR-024's kernel)"

# **The two git binaries a card carries**, from the Buildroot tree
# `tools/card/build.sh` takes them from — the same files, not a host build.
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
REPOSITORY_ROOT=$(constant "$REPO/rivet-core/src/git.rs" REPOSITORY_ROOT)
SERVED=$(constant "$REPO/rivet-fleet-repo/src/lib.rs" SERVED_NAME)
MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
ID_FILE=$(constant "$REPO/rivet-core/src/repo.rs" ID_FILE)
BARE=$REPOSITORY_ROOT/$SERVED.git
TAB=$(printf '\t')

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr035.XXXXXX") || fail "no working directory"
chmod 700 "$WORK"
QEMU_PID=
cleanup() {
    [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# **Nothing here may read this host's git configuration or credentials.**
HOME=$WORK/home
mkdir -p "$HOME"
export HOME
GIT_CONFIG_NOSYSTEM=1
GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_NOSYSTEM GIT_TERMINAL_PROMPT

# --- 1. the binaries --------------------------------------------------------

note "building the guest's binaries for $TARGET"
cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd \
    || fail "the guest build failed — see above"
note "building cargo-rivet for the host"
cargo build --release -p cargo-rivet || fail "cargo build -p cargo-rivet failed"
CARGO_RIVET=$HOST_OUT/cargo-rivet
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"

# --- 2. three throwaway identities, and a document declaring the repository --

GUEST_NAME=rivet-adr035-guest
mint() {
    "$CARGO_RIVET" identity new --directory "$WORK/$1" --name "$2" >"$WORK/mint-$1" 2>&1 \
        || { cat "$WORK/mint-$1" >&2; fail "could not mint the $1 identity"; }
    "$CARGO_RIVET" identity show --directory "$WORK/$1" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1
}
GUEST_PIN=$(mint secrets "$GUEST_NAME")
OPERATOR_PIN=$(mint operator rivet-adr035-operator)
STRANGER_PIN=$(mint stranger rivet-adr035-stranger)
[ -n "$GUEST_PIN" ] && [ -n "$OPERATOR_PIN" ] && [ -n "$STRANGER_PIN" ] || fail "an identity has no readable pin"
[ "$OPERATOR_PIN" != "$STRANGER_PIN" ] || fail "the operator and the stranger minted one key"

printf '# the operator this run minted, and nobody else\n%s\n' "$OPERATOR_PIN" \
    >"$WORK/secrets/${MCPD_PEERS_PATH##*/}" || fail "could not write peers"

# **Minted, never chosen**: sixteen hex characters from the OS, the grammar
# rivet_core::identifier accepts.
REPO_ID=$(openssl rand -hex 8)
DOC=$WORK/machine.toml
{
    printf 'version = 1\nhostname = "adr035-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[control-plane]\nlisten = "0.0.0.0:%s"\n' "$CONTROL_PORT"
    printf '\n[repos.%s]\nid = "%s"\n' "$SERVED" "$REPO_ID"
} >"$DOC" || fail "could not compose $DOC"

# --- 3. the machine ---------------------------------------------------------

CONSOLE=${RIVET_PROOF_LOG:-$REPO/out/adr-035-git-proof.console.log}
mkdir -p "$(dirname "$CONSOLE")"
: >"$CONSOLE"
note "booting with git and git-http-backend packed, [repos.$SERVED] declared"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --git "$GUEST_GIT" \
    --git-http-backend "$GUEST_HTTP_BACKEND" \
    --secrets "$WORK/secrets" \
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

# --- Q1: the supervisor created the repository ------------------------------

ATTEMPT=1
while :; do
    ask get file "$BARE/HEAD"
    [ "$ASK_RC" -eq 0 ] && grep -q 'refs/heads/main' "$WORK/ask-out" && break
    [ "$ATTEMPT" -ge 30 ] && { cat "$WORK/ask-out" "$WORK/ask-err" >&2; grep -a 'repos\|git' "$CONSOLE" | tail -20 >&2
        fail "$BARE/HEAD never named refs/heads/main — rivet-sysd did not create the declared repository"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done
echo "--- get file $BARE/HEAD, off the machine ---"
cat "$WORK/ask-out"
ask get file "$BARE/$ID_FILE"
echo "--- get file $BARE/$ID_FILE ---"
cat "$WORK/ask-out"
if [ "$ASK_RC" -eq 0 ] && grep -q "$REPO_ID" "$WORK/ask-out"; then
    note "the repository carries the identifier the document declared ($REPO_ID)"
else
    note "NOTE — $BARE/$ID_FILE did not read back as $REPO_ID through get file (see above)"
fi
note "Q1: rivet-sysd created $BARE on main"

# --- Q2: plan, then push ----------------------------------------------------

FLEET=$WORK/fleet
"$CARGO_RIVET" fleet init --at "$FLEET" >"$WORK/init.log" 2>&1 || { cat "$WORK/init.log" >&2; fail "fleet init"; }
[ "$(git -C "$FLEET" symbolic-ref --short HEAD)" = main ] || fail "fleet init did not name main"
mkdir -p "$FLEET/documents" "$FLEET/register/devices"
cp "$DOC" "$FLEET/documents/adr035-guest.provisioning.toml"
printf '# a register projection, for the proof\n' >"$FLEET/register/register.toml"
git -C "$FLEET" add -A
git -C "$FLEET" commit -q -m "record adr035-guest"
# **Forty small commits more**, so the clone in Q4 has history enough to make
# its fetch negotiation larger than git's 1 KiB gzip threshold.
_n=1
while [ "$_n" -le 40 ]; do
    printf 'revision %s\n' "$_n" >"$FLEET/register/revision.txt"
    git -C "$FLEET" add -A
    git -C "$FLEET" commit -q -m "revision $_n"
    _n=$((_n + 1))
done
LOCAL1=$(git -C "$FLEET" rev-parse HEAD)

push() {
    set +e
    "$CARGO_RIVET" fleet push --host "127.0.0.1:$CONTROL_PORT" --identity "$WORK/$1" --pin "$GUEST_PIN" \
        --at "$FLEET" $2 >"$WORK/push.log" 2>&1
    PUSH_RC=$?
    set -e
    echo "--- cargo rivet fleet push ($1${2:+ $2}) exited $PUSH_RC ---"
    cat "$WORK/push.log"
}

push operator ""
[ "$PUSH_RC" -eq 0 ] || fail "the plan exited $PUSH_RC"
grep -qF "create main, carrying 42 commit(s)" "$WORK/push.log" || fail "the plan did not say it would create main with 42 commits"
grep -qF "Nothing was pushed" "$WORK/push.log" || fail "the plan did not say nothing was pushed"
ask get file "$BARE/refs/heads/main"
if [ "$ASK_RC" -eq 0 ] && grep -q "$LOCAL1" "$WORK/ask-out"; then
    fail "the server's main already names $LOCAL1 after a plan"
fi

push operator confirm
[ "$PUSH_RC" -eq 0 ] || fail "the push exited $PUSH_RC"
grep -qF "PUSHED" "$WORK/push.log" || fail "the push did not report PUSHED"
grep -qF "$LOCAL1" "$WORK/push.log" || fail "the push did not name $LOCAL1"
note "Q2: a plan pushed nothing; confirm pushed $LOCAL1 and read it back"

# --- Q3: a second client clones it back, verifying TLS itself ---------------
#
# The operator's device.seed is raw Ed25519: PKCS#8 wraps it in sixteen fixed
# bytes (RFC 8410). THROWAWAY, and only ever inside $WORK.
pem_key() {
    { printf '\060\056\002\001\000\060\005\006\003\053\145\160\004\042\004\040'; cat "$WORK/$1/device.seed"; } \
        | openssl pkey -inform DER -out "$WORK/$1.key.pem" 2>"$WORK/pem-err" \
        || { cat "$WORK/pem-err" >&2; fail "could not wrap $1's seed as a PEM key"; }
    openssl x509 -inform DER -in "$WORK/$1/device.crt" -out "$WORK/$1.crt.pem" \
        || fail "could not read $1's certificate"
}
pem_key operator
pem_key stranger
openssl x509 -inform DER -in "$WORK/secrets/device.crt" -out "$WORK/guest.crt.pem" || fail "the guest's certificate"
GUEST_SPKI=$(openssl x509 -in "$WORK/guest.crt.pem" -pubkey -noout \
    | openssl pkey -pubin -outform DER | openssl dgst -sha256 -binary | openssl base64 -A)
URL="https://$GUEST_NAME:$CONTROL_PORT/$SERVED.git"

stock_git() {
    _who=$1
    shift
    git -c http.sslVerify=true \
        -c "http.sslCAInfo=$WORK/guest.crt.pem" \
        -c "http.pinnedPubkey=sha256//$GUEST_SPKI" \
        -c "http.sslCert=$WORK/$_who.crt.pem" \
        -c "http.sslKey=$WORK/$_who.key.pem" \
        -c "http.curloptResolve=$GUEST_NAME:$CONTROL_PORT:127.0.0.1" \
        -c credential.helper= \
        "$@"
}

stock_git operator clone -q "$URL" "$WORK/clone" 2>"$WORK/clone.err" \
    || { cat "$WORK/clone.err" >&2; fail "a stock git could not clone $URL over the control plane"; }
CLONED=$(git -C "$WORK/clone" rev-parse HEAD)
[ "$CLONED" = "$LOCAL1" ] || fail "the clone's HEAD is $CLONED, and the push sent $LOCAL1"
[ "$(git -C "$WORK/clone" rev-parse 'HEAD^{tree}')" = "$(git -C "$FLEET" rev-parse 'HEAD^{tree}')" ] \
    || fail "the cloned tree differs from the pushed one"
stock_git operator ls-remote "$URL" >"$WORK/ls-remote" 2>"$WORK/ls-remote.err" \
    || { cat "$WORK/ls-remote.err" >&2; fail "stock ls-remote"; }
echo "--- stock git ls-remote over the control plane ---"
cat "$WORK/ls-remote"
grep -q "^$LOCAL1${TAB}refs/heads/main\$" "$WORK/ls-remote" || fail "ls-remote does not name refs/heads/main at $LOCAL1"
grep -q "^$LOCAL1${TAB}HEAD\$" "$WORK/ls-remote" || fail "ls-remote does not name HEAD at $LOCAL1"
# Every tracked file's bytes, compared outside git's own object model — `git
# diff --no-index` is a plain two-path comparison (the container has no
# diffutils), and the two sets of paths are compared first.
git -C "$FLEET" ls-files >"$WORK/files-pushed"
git -C "$WORK/clone" ls-files >"$WORK/files-cloned"
git diff --no-index --exit-code "$WORK/files-pushed" "$WORK/files-cloned" >"$WORK/diff" 2>&1 \
    || { cat "$WORK/diff" >&2; fail "the clone tracks different paths from the pushed repository"; }
while IFS= read -r _path; do
    git diff --no-index --exit-code "$FLEET/$_path" "$WORK/clone/$_path" >"$WORK/diff" 2>&1 \
        || { cat "$WORK/diff" >&2; fail "$_path differs between the pushed repository and the clone"; }
done <"$WORK/files-pushed"
note "Q3: a stock git, verifying the guest's certificate and pin itself, cloned $CLONED; refs, tree and files match"

# --- Q4: a fast-forward, and a fetch with a gzipped negotiation --------------

printf 'version = 1\nhostname = "adr035-second"\n' >"$FLEET/documents/adr035-second.provisioning.toml"
git -C "$FLEET" add -A
git -C "$FLEET" commit -q -m "record adr035-second"
LOCAL2=$(git -C "$FLEET" rev-parse HEAD)
push operator ""
grep -qF "fast-forward main by 1 commit(s)" "$WORK/push.log" || fail "the second plan did not say fast-forward by 1"
push operator confirm
[ "$PUSH_RC" -eq 0 ] && grep -qF PUSHED "$WORK/push.log" || fail "the fast-forward push failed"

# The clone gains forty commits of its own the server has never seen, so its
# fetch sends rounds of `have` lines — over 1 KiB, which git gzips.
_n=1
while [ "$_n" -le 40 ]; do
    git -C "$WORK/clone" -c user.name=proof -c user.email=proof@rivet.invalid \
        commit -q --allow-empty -m "clone-only $_n"
    _n=$((_n + 1))
done
GIT_TRACE_CURL_NO_DATA=1 GIT_TRACE_CURL="$WORK/fetch.trace" \
    stock_git operator -C "$WORK/clone" fetch -q origin 2>"$WORK/fetch.err" \
    || { cat "$WORK/fetch.err" >&2; fail "the clone's fetch over the control plane failed"; }
[ "$(git -C "$WORK/clone" rev-parse origin/main)" = "$LOCAL2" ] || fail "the fetch did not bring $LOCAL2"
if grep -qi 'Content-Encoding: gzip' "$WORK/fetch.trace"; then
    note "Q4: the fetch's request body was gzipped, and the machine served it"
else
    note "Q4: NOTE — git did not gzip this fetch's request body, so that path is NOT exercised by this run"
fi
note "Q4: the second push fast-forwarded to $LOCAL2 and the clone fetched it"

# --- Q5: a caller whose key is not in peers ---------------------------------

push stranger confirm
[ "$PUSH_RC" -ne 0 ] || fail "a push by an identity not in peers exited 0"
grep -qF "control plane did not answer" "$WORK/push.log" || fail "the refusal does not name the control plane"
if stock_git stranger ls-remote "$URL" >"$WORK/stranger.out" 2>"$WORK/stranger.err"; then
    cat "$WORK/stranger.out" >&2
    fail "a stock git presenting an identity not in peers was served"
fi
echo "--- stock git as the stranger ---"
cat "$WORK/stranger.err"
stock_git operator ls-remote "$URL" refs/heads/main >"$WORK/ls-after" 2>"$WORK/ls-after.err" \
    || { cat "$WORK/ls-after.err" >&2; fail "the operator could not read main after the refusals"; }
grep -q "^$LOCAL2${TAB}" "$WORK/ls-after" || fail "the server's main moved after refused callers"
note "Q5: an identity not in peers was refused, by fleet push and by a stock git; main is unchanged"

# --- Q6: uncommitted work, and a push that is not a fast-forward -------------

printf 'half-written\n' >"$FLEET/documents/half.provisioning.toml"
push operator confirm
[ "$PUSH_RC" -ne 0 ] && grep -qF "uncommitted changes" "$WORK/push.log" || fail "uncommitted work was not refused"
rm -f "$FLEET/documents/half.provisioning.toml"

git -C "$WORK/clone" reset -q --hard origin/main
git -C "$WORK/clone" -c user.name=proof -c user.email=proof@rivet.invalid commit -q --allow-empty -m "someone else"
stock_git operator -C "$WORK/clone" push -q origin main 2>"$WORK/clone-push.err" \
    || { cat "$WORK/clone-push.err" >&2; fail "the second client could not push its own commit"; }
THEIRS=$(git -C "$WORK/clone" rev-parse HEAD)
printf 'version = 1\nhostname = "adr035-third"\n' >"$FLEET/documents/adr035-third.provisioning.toml"
git -C "$FLEET" add -A
git -C "$FLEET" commit -q -m "record adr035-third"
push operator confirm
[ "$PUSH_RC" -ne 0 ] && grep -qF "NOT A FAST-FORWARD" "$WORK/push.log" || fail "a push over someone else's commit was not refused"
stock_git operator ls-remote "$URL" refs/heads/main >"$WORK/ls-final" 2>"$WORK/ls-final.err" \
    || { cat "$WORK/ls-final.err" >&2; fail "the operator could not read main after the refused push"; }
grep -q "^$THEIRS${TAB}" "$WORK/ls-final" || fail "the refused push moved the server's main"
note "Q6: uncommitted work and a non-fast-forward were refused; the server kept $THEIRS"

echo
echo "adr-035-git-proof: PASS"
echo
echo "  What ran, on $RIVET_KERNEL under QEMU -M virt:"
echo "    * rivet-sysd created $BARE from [repos.$SERVED] in the guest's document;"
echo "    * cargo rivet fleet push planned (nothing sent), then pushed 42 commits"
echo "      over the control plane and read main back;"
echo "    * a stock git clone and ls-remote over the same port, verifying the guest's"
echo "      certificate and pinned key itself, matched refs, tree and files;"
echo "    * a second push fast-forwarded, and the clone fetched it;"
echo "    * an identity not in peers was refused by both clients;"
echo "    * uncommitted work and a non-fast-forward push were refused."
echo
echo "  What it does NOT settle: a board, a card's ext4 /state, rivet-netd's own"
echo "  network path, a realistic repository's size, or application objects."
echo "  The console is $CONSOLE."
