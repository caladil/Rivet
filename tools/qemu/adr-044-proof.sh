#!/bin/sh
# tools/qemu/adr-044-proof.sh — **fleet as code, proven both ways.**
#
# `ADR-044` §9, `.rulings/` R-145.3. Run it in the container:
#
#   RIVET_CACHE_TAG=-fleet tools/container/exec.sh tools/qemu/adr-044-proof.sh
#
# ## What it does
#
# Boots one aarch64 guest with a control plane and a declared `[repos.machines]`,
# pushes that guest's own document into the repository over the control plane
# with a stock git, and drives `rivet-fleetd converge` against the pair. The
# fleet server runs on the build host — the same code, a different
# architecture — and asks the guest's control plane for every git operation,
# because its register names the guest as `role = "fleet-server"`.
# `tools/qemu/adr-044-fleet-code-proof.sh` runs the same program confined, on
# the guest, as a fleet server really runs.
#
# ## Why BOTH halves, and why one alone is worthless
#
# `AGENTS.md` §4. A proof that only showed a push converging a machine cannot
# tell a working drift guard from a missing one: with the guard deleted, every
# assertion about convergence still passes. So this asserts, on one boot,
# against one machine:
#
# * a push that changes a document CONVERGES the machine, and the machine's own
#   `get desired-state` reports the new value; and
# * a push against a machine somebody has edited by hand is REFUSED, names the
#   drifted key with both values, and **leaves the machine exactly as it was** —
#   which is checked by reading the machine again, not by trusting the refusal.
#
# and three more that each settle something R-145 ruled:
#
# * the first cycle after a verbatim copy writes nothing and SAYS nothing;
# * a document carrying a credential is refused ON THE FLEET SERVER, by the
#   same guard that refuses an unknown key, before a machine is contacted;
# * the commit-message override turns the drift refusal into a write.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column, entirely. And two of its own:
#
# * **The fleet server here is an x86-64 build** run from a shell, not an
#   installed `.rvt` started by a supervisor with a socket and a Landlock
#   ruleset. What is exercised is the poller's logic and its control-plane
#   transport; `adr-044-fleet-code-proof.sh` is the confined half.
# * **The fleet server and the machine are one guest.** The repository and the
#   converged machine share a control plane.
#
# The guest has **no `/dev/null` and no `/dev/kmsg`** (`AGENTS.md` §10.5).
# Their absence in the console is the harness, not a finding.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
TARGET_TRIPLE=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO_ROOT/target` — `tools/container/exec.sh`
# mounts a named volume there, so a script that composed the in-tree path would
# build correctly and then look where nothing put the binaries.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO_ROOT/target}
OUT=$TARGET_DIR/$TARGET_TRIPLE/release
HOST_OUT=$TARGET_DIR/release

STAGE=
say() { echo "adr-044-proof: $*"; }
fail() {
    echo >&2
    echo "adr-044-proof: FAIL${STAGE:+ [$STAGE]}: $*" >&2
    exit 1
}

# **Zero-scrub.** `AGENTS.md` §8.3: nothing real goes into a tracked file, and
# a commit this makes is a commit in a throwaway repository under a temporary
# directory. The identity is fictional and says so.
GIT_AUTHOR_NAME="Rivet proof"
GIT_AUTHOR_EMAIL="proof@example.invalid"
GIT_COMMITTER_NAME=$GIT_AUTHOR_NAME
GIT_COMMITTER_EMAIL=$GIT_AUTHOR_EMAIL
export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL

command -v git >/dev/null 2>&1 \
    || fail "no git on PATH. This proof makes a repository and the poller reads one."
command -v od >/dev/null 2>&1 \
    || fail "no od on PATH — the operator's seed and certificate reach the fleet server as hex"
command -v openssl >/dev/null 2>&1 \
    || fail "no openssl on PATH — a stock git pushes to the guest with the operator's key as PEM"

# --- constants read out of the crates that own them -------------------------
#
# `smoke.sh`'s device, for its reason: a copy here goes stale silently and the
# failure looks like a guest that never came up.
constant() {
    value=$(sed -n "s/^pub const $2: *[a-z0-9&]* *= *\"\{0,1\}\([^\";]*\)\"\{0,1\};.*/\1/p" \
        "$1" | head -1)
    [ -n "$value" ] \
        || fail "cannot read $2 out of $1 — this harness has to agree with that crate about it and will not guess"
    printf '%s' "$value"
}
DEFINITIONS_RS="$REPO_ROOT/cmd/rivet-sysd/src/definitions.rs"
CONTROL_PLANE_RS="$REPO_ROOT/rivet-config/src/control_plane.rs"
MCPD_PEERS_PATH=$(constant "$DEFINITIONS_RS" MCPD_PEERS_PATH)
CONTROL_PORT=$(grep -oE 'DEFAULT_PORT: u16 = [0-9]+' "$CONTROL_PLANE_RS" | grep -oE '[0-9]+$')
[ -n "$CONTROL_PORT" ] || fail "could not read rivet_config::control_plane::DEFAULT_PORT"

# `MACHINE_DOCUMENT` is where a machine keeps its document, and this proof edits
# it out of band through `apply edit`, which takes the path.
MACHINE_DOCUMENT=$(constant "$REPO_ROOT/rivet-core/src/state.rs" MACHINE_DOCUMENT)
SERVER_ROLE=$(constant "$REPO_ROOT/rivet-core/src/fleet_as_code.rs" SERVER_ROLE)
REPOSITORY=machines
BR=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}
GUEST_GIT=$BR/target/usr/bin/git
GUEST_HTTP_BACKEND=$BR/target/usr/libexec/git-core/git-http-backend
[ -f "$GUEST_GIT" ] || fail "no vendored git at $GUEST_GIT — the guest serves the fleet's repository"
[ -f "$GUEST_HTTP_BACKEND" ] || fail "no git-http-backend at $GUEST_HTTP_BACKEND"

# --- the guest's binaries, and the fleet server's ---------------------------

CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || fail "no cross C compiler at $CC_aarch64_unknown_linux_musl.
    rivetos-mcpd links rivetos-tls, which links ring, whose build compiles C
    (ADR-009 3.7.4). Point RIVET_BR_OUTPUT at the Buildroot tree; in the
    container tools/container/exec.sh sets it to the /br mount."

say "building the guest for $TARGET_TRIPLE"
( cd "$REPO_ROOT" && cargo build --release --target "$TARGET_TRIPLE" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd ) \
    || fail "the guest's binaries did not build"

# **The fleet server and the operator's client, for the HOST.** `rivet-fleetd`
# is the thing under test and `cargo rivet run` is what reads the machine back —
# the binary an operator holds, not a probe written here, so a pass cannot mean
# something no real client could reproduce.
say "building the fleet server and cargo-rivet for the host"
( cd "$REPO_ROOT" && cargo build --release -p rivetos-fleetd -p cargo-rivet ) \
    || fail "the host binaries did not build"
FLEETD=$HOST_OUT/rivet-fleetd
CARGO_RIVET=$HOST_OUT/cargo-rivet
[ -x "$FLEETD" ] || fail "no fleet server at $FLEETD"
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr044.XXXXXX") || fail "no temporary directory"
chmod 700 "$WORK"
CONSOLE=$WORK/console.log
QEMU_PID=

cleanup() {
    if [ -n "$QEMU_PID" ]; then
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
    fi
    if [ "${RIVET_PROOF_KEEP:-no}" = yes ]; then
        echo "adr-044-proof: leaving $WORK in place (RIVET_PROOF_KEEP=yes)" >&2
    else
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

# --- two throwaway identities ------------------------------------------------
#
# Minted per run and never committed: a `device.seed` in this repository would
# be a private key every reader of it holds (`AGENTS.md` §8.3).
say "minting throwaway identities in $WORK"
"$CARGO_RIVET" identity new --directory "$WORK/guest" --name rivet-adr044-guest \
    >"$WORK/mint-guest" 2>&1 || { cat "$WORK/mint-guest" >&2; fail "could not mint the guest identity"; }
"$CARGO_RIVET" identity new --directory "$WORK/operator" --name rivet-adr044-operator \
    >"$WORK/mint-operator" 2>&1 || { cat "$WORK/mint-operator" >&2; fail "could not mint the operator identity"; }

GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/guest" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1)
OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 | grep -oE '[0-9a-f]{64}' | head -1)
[ -n "$GUEST_PIN" ] || fail "could not read the guest's public key back out of its certificate"
[ -n "$OPERATOR_PIN" ] || fail "could not read the operator's public key back out of its certificate"
[ "$GUEST_PIN" != "$OPERATOR_PIN" ] \
    || fail "the two identities minted the same key, so nothing below would tell one end from the other"

mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/"
printf '# one hex public key per line\n%s\n' "$OPERATOR_PIN" >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

# --- the guest's document ----------------------------------------------------
#
# `0.0.0.0` and not the built-in loopback default: SLIRP has no route to a
# loopback listener inside the guest, so a forwarded port would reach nothing
# and the failure would read as a control plane that never started.
DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "adr044-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
    printf '\n[repos.%s]\n' "$REPOSITORY"
    printf 'id = "%s"\n' "$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
} >"$DOC"
[ "$(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC")" -eq 1 ] \
    || fail "the composed document has more than one \`version\` key — check the fixture's line endings"

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

say "booting the guest, console to $CONSOLE"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" --cli "$OUT/rivet" --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" --secrets "$WORK/secrets" \
    --git "$GUEST_GIT" --git-http-backend "$GUEST_HTTP_BACKEND" \
    --document "$DOC" --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
    >"$CONSOLE" 2>&1 </dev/null &
QEMU_PID=$!

WAITED=0
while [ "$WAITED" -lt 240 ]; do
    if grep -qF 'rivet-mcpd: listening on' "$CONSOLE" 2>/dev/null; then
        break
    fi
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        cat "$CONSOLE" >&2
        fail "run.sh exited before the control plane reported listening — its console is above"
    fi
    sleep 0.5
    WAITED=$((WAITED + 1))
done
grep -qF 'rivet-mcpd: listening on' "$CONSOLE" \
    || { tail -40 "$CONSOLE" >&2; fail "the control plane never reported listening in 120s"; }
say "the guest is answering on 127.0.0.1:$CONTROL_PORT"

# --- what the operator's client does ----------------------------------------

ASKED=
ask() {
    set +e
    "$CARGO_RIVET" run --host "127.0.0.1:$CONTROL_PORT" \
        --identity "$WORK/operator" --pin "$GUEST_PIN" -- "$@" \
        >"$WORK/asked" 2>"$WORK/asked-err"
    ASK_RC=$?
    set -e
    ASKED=$(cat "$WORK/asked")
}

ask get desired-state
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/asked-err" >&2; fail "the guest would not answer \`get desired-state\`"; }
case $ASKED in
    *'hostname = \"adr044-guest\"'*) : ;;
    *) fail "the guest's document is not the one this harness planted: $ASKED" ;;
esac
say "the guest's own document is what was planted, read back over the control plane"

# --- the fleet's repository --------------------------------------------------

#
# **On the guest, served by its control plane.** `rivet-sysd` creates
# `machines.git` from the document's `[repos.machines]`; a stock git pushes to
# it over 7777 with the operator's throwaway key as PEM, pinning the guest's
# key itself, as `tools/qemu/adr-035-git-proof.sh` does.
CHECKOUT=$WORK/checkout
LABEL=adr044-guest
DEVICE_ID=00112233445566aa
GUEST_NAME=rivet-adr044-guest

ATTEMPT=1
while :; do
    ask get file "/state/git/$REPOSITORY.git/HEAD"
    [ "$ASK_RC" -eq 0 ] && case $ASKED in *refs/heads/main*) true ;; *) false ;; esac && break
    [ "$ATTEMPT" -ge 30 ] && fail "rivet-sysd never created /state/git/$REPOSITORY.git from [repos.$REPOSITORY]"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done

{ printf '\060\056\002\001\000\060\005\006\003\053\145\160\004\042\004\040'; cat "$WORK/operator/device.seed"; } \
    | openssl pkey -inform DER -out "$WORK/operator.key.pem" 2>/dev/null \
    || fail "could not wrap the operator's throwaway seed as a PEM key"
openssl x509 -inform DER -in "$WORK/operator/device.crt" -out "$WORK/operator.crt.pem" || fail "the operator's certificate"
openssl x509 -inform DER -in "$WORK/guest/device.crt" -out "$WORK/guest.crt.pem" || fail "the guest's certificate"
GUEST_SPKI=$(openssl x509 -in "$WORK/guest.crt.pem" -pubkey -noout \
    | openssl pkey -pubin -outform DER | openssl dgst -sha256 -binary | openssl base64 -A)
URL="https://$GUEST_NAME:$CONTROL_PORT/$REPOSITORY.git"

git init -q --initial-branch=main "$CHECKOUT"
mkdir -p "$CHECKOUT/fleet/machines"
cp "$REPO_ROOT/examples/fleet/README.md" "$CHECKOUT/README.md"

# **Verbatim.** `ADR-044` §4's bootstrap: the machine's current document is what
# goes in, so the first cycle finds them equal and adopts without writing.
cp "$DOC" "$CHECKOUT/fleet/machines/$LABEL.toml"

push_only() {
    git -C "$CHECKOUT" -c http.sslVerify=true -c "http.sslCAInfo=$WORK/guest.crt.pem" \
        -c "http.pinnedPubkey=sha256//$GUEST_SPKI" \
        -c "http.sslCert=$WORK/operator.crt.pem" -c "http.sslKey=$WORK/operator.key.pem" \
        -c "http.curloptResolve=$GUEST_NAME:$CONTROL_PORT:127.0.0.1" -c credential.helper= \
        push -q "$URL" main 2>"$WORK/push-err" \
        || { cat "$WORK/push-err" >&2; fail "a stock git could not push to the guest's $REPOSITORY repository"; }
}
push() {
    git -C "$CHECKOUT" add -A
    git -C "$CHECKOUT" commit -q -F "$1"
    push_only
}
printf 'The guest joins the fleet, verbatim\n' >"$WORK/msg"
push "$WORK/msg"

# --- the fleet server's own directory ---------------------------------------

APP_DATA=$WORK/appdata
mkdir -p "$APP_DATA/register/devices"
{
    printf 'name = "adr-044 proof"\n'
} >"$APP_DATA/register/register.toml"
{
    printf 'id = "%s"\n' "$DEVICE_ID"
    printf 'name = "%s"\n' "$LABEL"
    # The fleet server's own record: its control plane holds the repository.
    printf 'role = "%s"\n' "$SERVER_ROLE"
    printf 'pin = "%s"\n' "$GUEST_PIN"
    printf 'listen = "127.0.0.1:%s"\n' "$CONTROL_PORT"
    printf '\n[interfaces.eth0]\n'
    printf 'method = "static"\n'
    printf 'address = "127.0.0.1"\n'
} >"$APP_DATA/register/devices/$DEVICE_ID.toml"

# The operator identity the fleet server presents, as hex — `cmd/rivet-fleetd`'s
# `node::hex_file` and its own note on why these are hex.
hexof() { od -An -v -tx1 "$1" | tr -d ' \n'; }
hexof "$WORK/operator/device.seed" >"$APP_DATA/operator-device.seed.hex"
hexof "$WORK/operator/device.crt" >"$APP_DATA/operator-device.crt.hex"

SETTINGS=$WORK/settings
{
    printf 'fleet-repository=%s\n' "$REPOSITORY"
    printf 'fleet-branch=main\n'
} >"$SETTINGS"

CONVERGE_OUT=$WORK/converge.log
converge() {
    set +e
    RIVET_APP_DATA="$APP_DATA" \
    RIVET_APP_SETTINGS="$SETTINGS" \
        "$FLEETD" converge "$@" >"$CONVERGE_OUT" 2>&1
    CONVERGE_RC=$?
    set -e
}

expect_says() {
    grep -qF "$1" "$CONVERGE_OUT" \
        || { echo "--- what it said ---" >&2; cat "$CONVERGE_OUT" >&2; fail "expected the pass to say: $1"; }
}
expect_silent_about() {
    if grep -qF "$1" "$CONVERGE_OUT"; then
        echo "--- what it said ---" >&2; cat "$CONVERGE_OUT" >&2
        fail "the pass should NOT have said: $1"
    fi
}
guest_document() {
    ask get desired-state
    [ "$ASK_RC" -eq 0 ] || { cat "$WORK/asked-err" >&2; fail "the guest stopped answering"; }
}

# ===========================================================================
# STAGE 1 — the verbatim copy is adopted, and NOTHING is written or said
# ===========================================================================
STAGE="1 adoption is silent"
converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the first pass exited $CONVERGE_RC"; }
expect_says "the fleet agrees with the repository"
expect_silent_about "converging"
expect_silent_about "REFUSED"
say "STAGE 1 passed: $(cat "$CONVERGE_OUT")"

# ===========================================================================
# STAGE 2 — a second pass with nothing changed is quiet as well
# ===========================================================================
STAGE="2 an unchanged cycle is quiet"
converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the second pass exited $CONVERGE_RC"; }
expect_says "the fleet agrees with the repository"
expect_silent_about "converging"
say "STAGE 2 passed: a poller against a fleet that is already right says nothing"

# ===========================================================================
# STAGE 3 — a document carrying a credential is refused ON THE FLEET SERVER
# ===========================================================================
#
# `.rulings/` R-145.1 item 3, and the point is that this needed no new code: the
# schema has no key a credential could be written under, so the guard that
# refuses an unknown key is the guard that refuses a literal. It has to happen
# HERE and not on the machine, which is what the last assertion checks.
STAGE="3 a secret in the repository"
guest_document
BEFORE=$ASKED
# **Inside `[network.eth0]`, where a passphrase would really be written**, not
# appended at the end of the file where it would land in whichever table
# happens to be last.
sed -i 's|^gateway = "10.0.2.2"$|gateway = "10.0.2.2"\npsk = "a-credential-that-must-not-be-committed"|' \
    "$CHECKOUT/fleet/machines/$LABEL.toml"
grep -q '^psk = ' "$CHECKOUT/fleet/machines/$LABEL.toml" \
    || fail "the credential did not reach the file, so the refusal below would pass vacuously"
printf 'A wireless passphrase, which does not belong here\n' >"$WORK/msg"
push "$WORK/msg"
converge
[ "$CONVERGE_RC" -ne 0 ] || { cat "$CONVERGE_OUT" >&2; fail "a document carrying a credential was ACCEPTED"; }
expect_says "psk"
expect_says "install-secret"
guest_document
[ "$ASKED" = "$BEFORE" ] || fail "the machine changed while a bad document was being refused"
say "STAGE 3 passed: refused at the fleet server, and the machine was never contacted about it"

# Take it back out; every later stage works from a valid repository.
git -C "$CHECKOUT" checkout -q -- "fleet/machines/$LABEL.toml" 2>/dev/null || true
sed -i '/^psk = /d' "$CHECKOUT/fleet/machines/$LABEL.toml"
printf 'The passphrase comes back out; it belongs in apply install-secret\n' >"$WORK/msg"
push "$WORK/msg"
converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the repository is not valid again"; }

# ===========================================================================
# STAGE 4 — A PUSH CONVERGES THE MACHINE
# ===========================================================================
STAGE="4 a push converges a machine"
sed -i 's/^hostname = .*/hostname = "converged-by-a-push"/' "$CHECKOUT/fleet/machines/$LABEL.toml"
printf 'The guest is renamed by a push and by nothing else\n' >"$WORK/msg"
push "$WORK/msg"

converge --dry-run
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the dry run exited $CONVERGE_RC"; }
expect_says "WOULD DO"
expect_says "adr044-guest -> converged-by-a-push"
guest_document
case $ASKED in
    *'converged-by-a-push'*) fail "the DRY RUN wrote to the machine" ;;
esac
say "STAGE 4a passed: --dry-run named the change and touched nothing"

converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the converging pass exited $CONVERGE_RC"; }
expect_says "converging onto the repository's document"
expect_says "is converged onto"
guest_document
case $ASKED in
    *'hostname = \"converged-by-a-push\"'*) : ;;
    *) fail "the machine's own document does not carry the pushed value: $ASKED" ;;
esac
say "STAGE 4 passed: A COMMIT PUSHED TO A REPOSITORY CHANGED A MACHINE"

converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the pass after the write exited $CONVERGE_RC"; }
expect_says "the fleet agrees with the repository"
expect_silent_about "converging"
say "STAGE 4b passed: and the cycle after it is silent again"

# ===========================================================================
# STAGE 5 — A PUSH THAT WOULD OVERWRITE DRIFT IS REFUSED
# ===========================================================================
#
# The half without which stage 4 proves nothing about the guard.
STAGE="5 a push over drift is refused"
# **Two kinds of drift, because the report has two shapes.** A key whose value
# moved has two values to print; a table somebody added has none on the
# repository's side, and it is the one an implementation forgets — the
# comparison that answers *what would this write take away* cannot see it at
# all.
ask apply edit "$MACHINE_DOCUMENT" \
    --expect 'hostname = "converged-by-a-push"' \
    --replace 'hostname = "changed-by-hand"' confirm
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/asked-err" "$WORK/asked" >&2; fail "could not change a value on the machine out of band"; }
ADDED=$(printf '\n[kiosk]\nauto-run = "changed by hand, out of band"\n')
ask apply edit "$MACHINE_DOCUMENT" --append "$ADDED" confirm
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/asked-err" "$WORK/asked" >&2; fail "could not add a table to the machine out of band"; }
guest_document
case $ASKED in
    *'changed-by-hand'*) : ;;
    *) fail "the out-of-band change did not reach the machine's document: $ASKED" ;;
esac
case $ASKED in
    *'out of band'*) : ;;
    *) fail "the out-of-band addition did not reach the machine's document: $ASKED" ;;
esac
DRIFTED=$ASKED
say "the machine has been edited out of band, as an operator with apply edit would"

sed -i 's/^hostname = .*/hostname = "pushed-over-drift"/' "$CHECKOUT/fleet/machines/$LABEL.toml"
printf 'A push that arrives after somebody edited the machine\n' >"$WORK/msg"
push "$WORK/msg"

converge
[ "$CONVERGE_RC" -ne 0 ] \
    || { cat "$CONVERGE_OUT" >&2; fail "the push over drift was ACCEPTED. This is the guard missing."; }
expect_says "REFUSED"
expect_says "NOTHING HAS BEEN WRITTEN"
# **The key and both values**, which is the difference between a report an
# operator can act on and one they cannot. Both shapes: the value that moved,
# with the repository's and the machine's side of it, and the table that was
# added, with the value it was added with.
expect_says "hostname"
expect_says "repository  converged-by-a-push"
expect_says "machine     changed-by-hand"
expect_says "kiosk.auto-run"
expect_says "machine     changed by hand, out of band"
expect_says "Fleet-Override: overwrite-drift adr044-guest"
guest_document
[ "$ASKED" = "$DRIFTED" ] \
    || fail "the machine changed during a refusal. NOTHING HAS BEEN WRITTEN is a lie."
case $ASKED in
    *'pushed-over-drift'*) fail "the refused document reached the machine" ;;
esac
say "STAGE 5 passed: REFUSED, named the drifted key with both values, and the machine is untouched"
# **Printed, not only asserted.** The greps above check that the words are
# there; a person reading a proof run has to be able to see whether the report
# is one they could act on.
echo "--- the refusal, verbatim ---"
cat "$CONVERGE_OUT"
echo "--- end of the refusal ---"

# ===========================================================================
# STAGE 6 — the commit-message override lets the push win
# ===========================================================================
STAGE="6 the override word"
{
    printf 'The repository wins this one\n'
    printf '\n'
    printf 'Fleet-Override: overwrite-drift %s\n' "$LABEL"
} >"$WORK/msg"
# An empty commit: the document in the repository is already what should win,
# and what changes is the PUSH — which is exactly what the trailer belongs to.
git -C "$CHECKOUT" commit -q --allow-empty -F "$WORK/msg"
push_only

converge
[ "$CONVERGE_RC" -eq 0 ] || { cat "$CONVERGE_OUT" >&2; fail "the overriding pass exited $CONVERGE_RC"; }
expect_says "overwrite-drift"
expect_says "OVERWRITING"
guest_document
case $ASKED in
    *'hostname = \"pushed-over-drift\"'*) : ;;
    *) fail "the override did not write the repository's document: $ASKED" ;;
esac
case $ASKED in
    *'out of band'*) fail "the override wrote the document and the drift survived, so it was not a whole-document write" ;;
esac
say "STAGE 6 passed: the trailer named this machine, the repository won, and the report said what it overwrote"

# ===========================================================================
# STAGE 7 — a machine that is off is not a failure
# ===========================================================================
STAGE="7 an unreachable machine"
# **A second machine, and the guest stays up.** The guest is this fleet's
# server as well as its machine, and its control plane holds the repository, so
# switching it off stops the pass itself. The machine that is off is one the
# register places where nothing listens: loopback, port 1.
OFF_ID=00112233445566bb
{
    printf 'id = "%s"\nname = "garage-pi"\npin = "%s"\nlisten = "127.0.0.1:1"\n' "$OFF_ID" "$GUEST_PIN"
    printf '\n[interfaces.eth0]\nmethod = "static"\naddress = "127.0.0.1"\n'
} >"$APP_DATA/register/devices/$OFF_ID.toml"
printf 'version = 1\nhostname = "nobody-is-listening"\n' >"$CHECKOUT/fleet/machines/garage-pi.toml"
printf 'A push at a machine that is switched off\n' >"$WORK/msg"
push "$WORK/msg"
converge
expect_says "not reachable this cycle, which is not a failure"
expect_silent_about "REFUSED"
[ "$CONVERGE_RC" -eq 0 ] \
    || { cat "$CONVERGE_OUT" >&2; fail "an unreachable machine exited non-zero. It is a state, not a failure, and an operator who is paged for it will stop reading."; }
say "STAGE 7 passed: $(cat "$CONVERGE_OUT")"

echo
expect_says "garage-pi"
ask apply power-off confirm || true
say "PASSED — both halves, on one boot, against one machine:"
say "  a push converged it, and the machine's own get desired-state said so;"
say "  a push over drift was refused, named the key and both values, and wrote nothing."
say "It settles nothing in ADR-026 section 4's right-hand column, and the fleet"
say "server here is a host build driven from a shell rather than an installed"
say ".rvt under a supervisor. See this file's header."
