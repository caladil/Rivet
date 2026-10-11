#!/bin/sh
# tools/qemu/adr-052-proof.sh — **the artifact store, exercised with real
# signed artifacts on a booted machine.**
#
# `ADR-052` §7.2, and the half of it that four commits on 2026-09-12 built and
# did not demonstrate. Run it in the container:
#
#   RIVET_APP_SEED=$HOME/.rivet/keys/rivet-app.seed \
#       tools/container/exec.sh tools/qemu/adr-052-proof.sh
#
# ## What was missing, stated plainly
#
# `ea321a63`, `b7d1d943`, `1b4cc906` and `c30ef19c` added a digest-keyed store,
# changed the install operation to name a digest, and repaired the harness flag
# that writes an install request. The only QEMU runs behind them used
# `run.sh --deploy-request`, which packs **a placeholder object** — a line of
# text whose contents are not the digest that names it — and did not pack
# `cmd/rivet-deploy` at all. So the supervisor's *reading* half was watched on a
# real kernel and **nothing had verified a signature out of the store**, which
# is the one thing the store exists to lead to.
#
# This script closes that. Nothing here is packed into an initramfs by hand:
# every artifact arrives **over the control plane, through `ADR-035` §3.2's bulk
# endpoint**, keyed by the digest that endpoint computed, and is installed by a
# `cmd/rivet-deploy` the supervisor started from a request naming that digest.
#
# ## The four questions it asks
#
# 1. **A real `.rvt` reaches the store and its signature is VERIFIED out of it.**
#    `cargo rivet package` seals it with the application seed; the guest's trust
#    store carries the public half from `keys/rivet-app.pub`; `cmd/rivet-deploy`
#    opens the object the store resolved and checks it. Installed, and then
#    ADOPTED — the supervisor starts the application and its own lines appear on
#    the console.
# 2. **An artifact whose signature does not verify is REFUSED AT THE DOOR** —
#    by the bulk endpoint, before it is named in the store (`ADR-052` §7.2.3,
#    since 2026-09-12). It is signed by a key this machine does not trust, so the
#    digest is honest and the *signature* is the thing that fails — which is the
#    branch that matters. A corrupted file would fail a different check. The
#    deploy must say the machine refused the ARTIFACT rather than that the
#    transfer failed, the machine's log must say why, and the digest must be in
#    neither `objects/` nor `incoming/`. Until that date the installer was what
#    refused it, after it had reached the store.
#    **And one TRUSTED artifact whose header says `armv6`** is refused at the
#    same door against the `aarch64` the guest's `/system/arch` records
#    (`PLAN-Feature-Set-2026-09-12` §3.3g) — correctly signed, so the only thing
#    that can refuse it is the architecture.
# 3. **Retention keeps the three newest of each product**, against a real
#    filesystem in a booted guest rather than a temporary directory: four
#    versions of one product arrive, one is evicted, and the object list is read
#    back off the machine with `get file`.
# 4. **The artifact that has just arrived is never the one dropped.** `b7d1d943`
#    found this by unit test before anything ran; here the fourth arrival's own
#    install is asked for immediately afterwards, which is exactly the sequence
#    that would have failed.
#
# ## What it does NOT settle
#
# **`/state` is the initramfs tmpfs, not ext4 on a card.** `run.sh --card` gives
# a guest a real GPT and a real ext4 — and refuses `--document`, `--secrets` and
# `--state` with it, because PID 1 mounts the card over the directory all three
# write into. A control plane needs an identity and a document, so a run with
# `--card` cannot receive a transfer at all and this proof cannot have both.
# What is real here is the *filesystem calls*: `rename(2)` of a directory,
# `readdir`, `mtime` in whole seconds, `remove_dir_all` — on the target kernel,
# in a booted machine. What is not real is the medium.
#
# **tmpfs makes the tie MORE likely, not less**, which is the right direction:
# `Stored::added` is seconds, so artifacts arriving inside one second tie, and
# the tie is what `evictions`' `arriving` argument exists for.
#
# **QEMU is not a board** (`ADR-026` §4). Nothing here says anything about SD
# timing, the firmware, or a slot write.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO/target`, for `adr-042-proof.sh`'s reason:
# the container mounts a named volume there and exports it.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release
HOST_OUT=$TARGET_DIR/release

# **Every crate this script builds lives OUTSIDE the workspace**, in a temporary
# directory, so none of them inherits `.cargo/config.toml`'s cross linker.
# `rust-lld` ships with the toolchain (`ADR-026` §3.1) and naming it here is the
# same fact that file states — said once, for every crate below, rather than
# copied into each temporary crate directory.
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER

# **And a cross C COMPILER, because `rivetos-mcpd` links `ring`** (`ADR-009`
# §3.7.4). `smoke.sh` does exactly this and refuses early by name without it;
# the same refusal here, because the alternative is a `cc-rs` error about a tool
# nobody named that reads as a broken toolchain.
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
export CC_aarch64_unknown_linux_musl

fail() { echo; echo "adr-052-proof: FAIL: $*" >&2; exit 1; }
note() { echo "adr-052-proof: $*"; }

command -v qemu-system-aarch64 >/dev/null 2>&1 \
    || fail "qemu-system-aarch64 is not on PATH. tools/container/exec.sh puts it there."
command -v timeout >/dev/null 2>&1 \
    || fail "coreutils 'timeout' is not on PATH, and this run is bounded"
[ -x "$CC_aarch64_unknown_linux_musl" ] \
    || fail "no cross C compiler at $CC_aarch64_unknown_linux_musl.
    rivetos-mcpd links rivetos-tls, which links ring, whose build compiles C.
    Point RIVET_BR_OUTPUT at the Buildroot tree that holds host/bin, or set
    CC_aarch64_unknown_linux_musl yourself. tools/container/exec.sh mounts it
    at /br and exports RIVET_BR_OUTPUT for you."

# The kernel. `exec.sh` mounts the Buildroot output at /br and does not export
# RIVET_KERNEL; `adr-042-proof.sh` fills it in the same way and for the same
# reason — a caller should not have to remember which wrapper sets what.
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi
[ -n "${RIVET_KERNEL:-}" ] || fail "no RIVET_KERNEL and no /br/images/Image.
    --net needs ADR-024's own kernel: the Raspberry Pi's kernel8.img has no
    virtio-net driver at all and run.sh refuses it by name."

# --- the constants this harness must not spell for itself -------------------
#
# `smoke.sh`'s device, and its argument: a third spelling of a path or a port in
# a shell script is exactly what `ADR-011` is about.
constant() {
    _file=$1
    _name=$2
    _value=$(sed -n "s/^pub const $_name: *[a-z0-9&]* *= *\"\{0,1\}\([^\";]*\)\"\{0,1\};.*/\1/p" \
        "$_file" | head -1)
    [ -n "$_value" ] \
        || fail "cannot read $_name out of $_file — this harness has to agree with that crate and will not guess"
    printf '%s\n' "$_value"
}

CONTROL_PORT=$(constant "$REPO/rivet-config/src/control_plane.rs" DEFAULT_PORT)
ARTIFACTS_DIR=$(constant "$REPO/rivet-core/src/state.rs" ARTIFACTS)
KEEP=$(sed -n 's/^pub const KEEP: usize = \([0-9]*\);.*/\1/p' "$REPO/rivet-core/src/artifact.rs" | head -1)
[ -n "$KEEP" ] || fail "cannot read rivet_core::artifact::KEEP — the retention count is that crate's, not this file's"
OBJECTS_DIR=$ARTIFACTS_DIR/objects

# --- where the run's rubbish goes -------------------------------------------

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr052.XXXXXX") \
    || fail "could not create a working directory"
chmod 700 "$WORK"
QEMU_PID=
cleanup() {
    [ -n "$QEMU_PID" ] && kill "$QEMU_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# --- 1. the binaries --------------------------------------------------------

note "building the guest's binaries for $TARGET"
cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd \
    -p rivetos-mcpd -p rivetos-deploy \
    || fail "the guest build failed — see above"

note "building cargo-rivet for the HOST: the packager and the control-plane client"
cargo build --release -p cargo-rivet || fail "cargo build -p cargo-rivet failed"
CARGO_RIVET=$HOST_OUT/cargo-rivet
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"

for _b in rivet-initd rivet-sysd rivet rivet-netd rivet-mcpd rivet-deploy; do
    [ -f "$OUT/$_b" ] || fail "the build left no $_b at $OUT/$_b"
done

note "building the fixture signer, which writes the trust store"
cargo build --release --manifest-path "$HERE/fixture-signer/Cargo.toml" \
    || fail "the fixture signer would not build"
FIXTURE_SIGNER=$HOST_OUT/rivet-fixture-signer
[ -x "$FIXTURE_SIGNER" ] || fail "no rivet-fixture-signer at $FIXTURE_SIGNER"

# --- 2. the trust store the guest will carry --------------------------------
#
# **The real public key, from `keys/rivet-app.pub`** — the one
# `tools/card/build.sh` registers, so what this guest trusts is what a card
# built from a clean checkout trusts. The store file is written by
# `rivet_core::package::trust_store`, never by this script: `run.sh --trust`'s
# own help refuses to compose one and it is right to.
TRUST=$WORK/trust
"$FIXTURE_SIGNER" trust-store "$TRUST" "rivet-app:application:$REPO/keys/rivet-app.pub" \
    || fail "could not write the trust store"

# --- 3. the artifacts -------------------------------------------------------
#
# **Real `.rvt` files from `cargo rivet package`**, sealed with the application
# seed. Nothing here writes a package format, a header or a signature.
#
# The seed is found the way that tool documents: `$RIVET_APP_SEED`, then
# `~/.rivet/keys/rivet-app.seed`. `tools/container/exec.sh` mounts one read-only
# when the caller names it on the host. This script never reads it and never
# prints it.

# A crate copied out of the tree, at a version this script chooses.
#
# **Out of the tree deliberately.** `rivet_remote::name::Version::compose` asks
# `git` in the crate's own directory, and a copy in a temporary directory is not
# a work tree — so the version is the bare one from `Cargo.toml`, with no
# `-g<short>` and no `-dirty`. That makes four artifacts whose names differ by
# exactly the field this is varying, which is what a retention rule that groups
# by product is supposed to be given.
stage_crate() {
    _src=$1
    _dest=$2
    _version=$3
    mkdir -p "$_dest"
    cp -a "$_src/Cargo.toml" "$_src/src" "$_dest/" \
        || fail "could not stage $_src at $_dest"
    # The version, and only the version. `[package]`'s is the first `version =`
    # in the file; a `[dependencies]` entry with one would come later.
    sed -i "0,/^version = /s/^version = .*/version = \"$_version\"/" "$_dest/Cargo.toml" \
        || fail "could not set the version in $_dest/Cargo.toml"
    grep -q "^version = \"$_version\"\$" "$_dest/Cargo.toml" \
        || fail "the version in $_dest/Cargo.toml is not $_version — the substitution missed"
}

# `cargo rivet package`, and the BLAKE3 it prints.
#
# The digest is read out of the tool's own report rather than computed here:
# this script has no hasher and should not acquire one (`ADR-011`), and the
# number that matters is the one the packager and the board will agree about.
package() {
    _crate=$1
    _report=$2
    shift 2
    "$CARGO_RIVET" package \
        --manifest-path "$_crate/Cargo.toml" \
        --target "$TARGET" \
        --target-dir "$_crate/target" \
        --out-dir "$_crate/rvt" \
        --no-assets \
        "$@" \
        >"$_report" 2>&1 \
        || { cat "$_report" >&2; fail "cargo rivet package failed for $_crate"; }
}

rvt_of() {
    _dir=$1
    _found=$(find "$_dir/rvt" -name '*.rvt' -type f | head -1)
    [ -n "$_found" ] || fail "no .rvt under $_dir/rvt"
    printf '%s\n' "$_found"
}

blake3_of() {
    _d=$(sed -n 's/^blake3   *\([0-9a-f]\{64\}\).*/\1/p' "$1" | head -1)
    [ -n "$_d" ] || { cat "$1" >&2; fail "cargo rivet package printed no blake3 line"; }
    printf '%s\n' "$_d"
}

HELLO_SRC=$REPO/examples/probes/hello-rivet
OTHER_SRC=$REPO/examples/probes/slow-stop-probe
[ -d "$HELLO_SRC" ] || fail "no crate at $HELLO_SRC"
[ -d "$OTHER_SRC" ] || fail "no crate at $OTHER_SRC"

note "packaging four versions of hello-rivet and one of slow-stop-probe"
V1=0.1.1
V2=0.1.2
V3=0.1.3
V4=0.1.4
for _v in $V1 $V2 $V3 $V4; do
    stage_crate "$HELLO_SRC" "$WORK/hello-$_v" "$_v"
    package "$WORK/hello-$_v" "$WORK/package-hello-$_v.log"
done
stage_crate "$OTHER_SRC" "$WORK/other" 1.0.0
package "$WORK/other" "$WORK/package-other.log"

# **And one artifact this machine must REFUSE.**
#
# Signed by a key the guest's trust store does not carry, so the transfer is
# honest — the digest is the hash of the bytes, computed by the endpoint — and
# the *signature* is what fails. That is the branch worth watching: a truncated
# or corrupted file fails the transfer's own digest check and never reaches the
# store's door, let alone an installer.
#
# **The key is thirty-two identical bytes and it is not a secret.**
# `tools/qemu/fixture-signer` has argued this already and its constant is the
# same one: it signs nothing any machine outside this harness trusts, and the
# whole point here is that the guest does NOT trust it.
UNTRUSTED_KEY=$WORK/untrusted-key.hex
printf '2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a\n' \
    >"$UNTRUSTED_KEY"
stage_crate "$HELLO_SRC" "$WORK/hello-untrusted" 0.9.9
package "$WORK/hello-untrusted" "$WORK/package-untrusted.log" \
    --signing-key "$UNTRUSTED_KEY"

RVT_V1=$(rvt_of "$WORK/hello-$V1"); DIG_V1=$(blake3_of "$WORK/package-hello-$V1.log")
RVT_V2=$(rvt_of "$WORK/hello-$V2"); DIG_V2=$(blake3_of "$WORK/package-hello-$V2.log")
RVT_V3=$(rvt_of "$WORK/hello-$V3"); DIG_V3=$(blake3_of "$WORK/package-hello-$V3.log")
RVT_V4=$(rvt_of "$WORK/hello-$V4"); DIG_V4=$(blake3_of "$WORK/package-hello-$V4.log")
RVT_OTHER=$(rvt_of "$WORK/other");  DIG_OTHER=$(blake3_of "$WORK/package-other.log")
RVT_BAD=$(rvt_of "$WORK/hello-untrusted")
DIG_BAD=$(blake3_of "$WORK/package-untrusted.log")

# **And one artifact built for ANOTHER ARCHITECTURE** —
# `PLAN-Feature-Set-2026-09-12` §3.3g.
#
# Sealed with the REAL application seed, so its signature verifies and the only
# thing wrong with it is what it is for. Its header says `armv6`, which is what
# `cargo rivet package` writes for `arm-unknown-linux-musleabihf`; the program
# inside is hello-rivet's aarch64 build, handed over with `--binary`, because
# this container carries no armv6 toolchain and the door never looks past the
# header's claim. A board that ran this program would run it — which is exactly
# why the claim, and not the bytes, is what has to be refused: an artifact that
# SAYS it is for another machine is garbage whether or not it happens to be.
HELLO_BIN=$WORK/hello-$V1/target/$TARGET/release/hello-rivet
[ -f "$HELLO_BIN" ] || fail "no hello-rivet binary at $HELLO_BIN to package as armv6"
stage_crate "$HELLO_SRC" "$WORK/hello-armv6" 0.1.5
package "$WORK/hello-armv6" "$WORK/package-armv6.log" \
    --target arm-unknown-linux-musleabihf --binary "$HELLO_BIN"
RVT_ARCH=$(rvt_of "$WORK/hello-armv6")
DIG_ARCH=$(blake3_of "$WORK/package-armv6.log")
case "$(basename "$RVT_ARCH")" in
    *-armv6-application.rvt) ;;
    *) fail "the other-architecture artifact is named $(basename "$RVT_ARCH"), not an armv6 application" ;;
esac

# Seven distinct digests, or the retention assertions below would be counting
# the same object more than once and would pass vacuously.
_count=$(printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n' \
    "$DIG_V1" "$DIG_V2" "$DIG_V3" "$DIG_V4" "$DIG_OTHER" "$DIG_BAD" "$DIG_ARCH" | sort -u | wc -l)
[ "$_count" -eq 7 ] \
    || fail "the seven artifacts produced $_count distinct digests. Retention groups by product and counts objects; duplicates would make the count below mean nothing."

echo "--- what was packaged ---"
for _f in "$RVT_V1" "$RVT_V2" "$RVT_V3" "$RVT_V4" "$RVT_OTHER" "$RVT_BAD" "$RVT_ARCH"; do
    printf '  %s  %s bytes\n' "$(basename "$_f")" "$(wc -c <"$_f" | tr -d ' ')"
done

# --- 4. the machine ---------------------------------------------------------
#
# `smoke.sh`'s control-plane stage, narrowed. Two throwaway TLS identities, a
# document that gives the guest an address and a listener, and a boot with the
# installer packed.
note "minting THROWAWAY identities in $WORK"
"$CARGO_RIVET" identity new --directory "$WORK/secrets" --name rivet-adr052-guest \
    >"$WORK/mint-guest" 2>&1 \
    || { cat "$WORK/mint-guest" >&2; fail "could not mint the guest's identity"; }
"$CARGO_RIVET" identity new --directory "$WORK/operator" --name rivet-adr052-operator \
    >"$WORK/mint-operator" 2>&1 \
    || { cat "$WORK/mint-operator" >&2; fail "could not mint the operator's identity"; }

# **Minted straight into the directory `--secrets` will place**, rather than
# minted elsewhere and copied. `smoke.sh` copies, and `run.sh`'s own refusal
# text recommends this spelling; it also means nothing in this script ever
# opens, moves or copies a private key.
GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/secrets" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
[ -n "$GUEST_PIN" ] || fail "could not read the guest's pin out of its certificate"
[ -n "$OPERATOR_PIN" ] || fail "could not read the operator's pin out of its certificate"
[ "$GUEST_PIN" != "$OPERATOR_PIN" ] \
    || fail "the two identities minted the same public key, so nothing below tells one end from the other"

MCPD_PEERS_PATH=$(constant "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH)
PEERS_FILE=$WORK/secrets/${MCPD_PEERS_PATH##*/}
printf '# one hex public key per line — the operator this run minted\n%s\n' \
    "$OPERATOR_PIN" >"$PEERS_FILE" \
    || fail "could not write the guest's pin file"

DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "adr052-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC" || fail "could not compose $DOC"
[ "$(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC")" -eq 1 ] \
    || { cat -n "$DOC" >&2; fail "the composed document has more than one \`version\` key"; }

# **Outside `$WORK`, which this script removes on exit, and outside the
# container's `/tmp`, which goes with the container.**
#
# A failing run whose only evidence went with the temporary directory is a run
# that has to be done again to be read — and `tools/container/exec.sh` forwards
# no variable a caller could redirect this with, so a default under `$TMPDIR`
# would be unreachable from the host that started the run. `out/` is this
# repository's untracked artifacts directory and is mounted, so the console
# lands where the person who typed the command can read it.
CONSOLE=${RIVET_PROOF_LOG:-$REPO/out/adr-052-proof.console.log}
mkdir -p "$(dirname "$CONSOLE")" || fail "cannot create $(dirname "$CONSOLE")"
: >"$CONSOLE"

# **Every assertion about the console is made on the lines that arrived AFTER a
# mark**, and this is not tidiness.
#
# `ADR-022` §4 gives a record more than one way to the console: the installer
# writes a record on stdout, the supervisor captures it and re-emits it wrapped,
# and the kernel ring buffer is a third path — the same sentence can appear two
# or three times on one boot. A run that counted occurrences would read the
# FIRST install's success as the second install's, which is exactly the wrong
# answer for the artifact that must be refused. So: mark, act, and read only
# what is new.
mark() { wc -l <"$CONSOLE" | tr -d ' '; }
since() { tail -n "+$(($1 + 1))" "$CONSOLE"; }
# Wait until a pattern appears after a mark, or give up. Prints nothing.
await() {
    _mark=$1
    _pattern=$2
    _limit=$3
    _waited=0
    while [ "$_waited" -lt "$_limit" ]; do
        if since "$_mark" | grep -aq "$_pattern"; then
            return 0
        fi
        sleep 0.5
        _waited=$((_waited + 1))
    done
    return 1
}
note "booting: rivet-deploy packed as the installer, $TRUST as the trust store,"
note "         the control plane on 0.0.0.0:$CONTROL_PORT, host $CONTROL_PORT forwarded"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --deploy "$OUT/rivet-deploy" \
    --secrets "$WORK/secrets" \
    --trust "$TRUST" \
    --document "$DOC" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
    >"$CONSOLE" 2>&1 </dev/null &
QEMU_PID=$!

WAITED=0
while [ "$WAITED" -lt 240 ]; do
    grep -qF 'rivet-mcpd: listening on' "$CONSOLE" 2>/dev/null && break
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        cat "$CONSOLE" >&2
        fail "run.sh exited before the control plane reported listening — its console is above"
    fi
    sleep 0.5
    WAITED=$((WAITED + 1))
done
grep -qF 'rivet-mcpd: listening on' "$CONSOLE" \
    || { tail -60 "$CONSOLE" >&2; fail "the control plane never reported listening within 120s"; }
note "the control plane is listening"

# --- 5. the client ----------------------------------------------------------

# **Every deploy publishes into a fleet repository first** (`ADR-052` §7), so
# this run gives it a SCRATCH one inside `$WORK` and never the default
# `~/.rivet/fleet`. The publish verifies against the application seed found by
# `cargo rivet package`'s lookup — `$RIVET_APP_SEED` here. `DEPLOY_SEED` names a
# different seed for one deploy: the untrusted artifact below is published into
# its own scratch repository against the key that signed it, so it reaches the
# machine's door, which is the refusal this proof watches.
DEPLOY_FLEET=$WORK/fleet
DEPLOY_SEED=
deploy() {
    _rvt=$1
    _log=$2
    shift 2
    set +e
    env ${DEPLOY_SEED:+RIVET_APP_SEED=$DEPLOY_SEED} "$CARGO_RIVET" deploy \
        --host "127.0.0.1:$CONTROL_PORT" \
        --identity "$WORK/operator" \
        --pin "$GUEST_PIN" \
        --at "$DEPLOY_FLEET" \
        --rvt "$_rvt" \
        "$@" \
        confirm >"$_log" 2>&1
    DEPLOY_RC=$?
    set -e
}

# **Deploy, then WAIT for the machine to finish with it before deploying
# again**, and this is a fact about Rivet rather than a nicety of this script.
#
# `rivet_core::deploy`'s request file is one slot. `rivet apply install` refuses
# a second request while one is still waiting —
#
#   REFUSED an install request is already waiting at /state/deploy-request and
#   NOTHING HAS BEEN DELIVERED.
#
# — which is correct: two installers would be two writers on one inactive slot.
# Found here on 2026-09-12, deploying four versions back to back and losing the
# second.
#
# **`cargo rivet deploy` waits that refusal out now** (`ADR-052` §7.2.7), so the
# deploys no longer race each other. This wait is a DIFFERENT one and it is still
# needed: the tool waits for the request SLOT to clear, which happens when the
# supervisor takes the request up — not when the installer has finished with it.
# Every assertion below is about what the installer then did, so what this waits
# for is the INSTALLER's own record, and what comes next starts as soon as the
# machine is actually done.
deploy_and_settle() {
    _settle_mark=$(mark)
    _settle_rvt=$1
    _settle_log=$2
    shift 2
    deploy "$_settle_rvt" "$_settle_log" "$@"
    [ "$DEPLOY_RC" -eq 0 ] || return 0
    await "$_settle_mark" 'the application is installed' 240 \
        || { since "$_settle_mark" | grep -a 'rivet-deploy' | tail -20 >&2
             fail "$(basename "$_settle_rvt") was transferred and the installer never finished with it. The console is $CONSOLE."; }
}

ask() {
    set +e
    "$CARGO_RIVET" run \
        --host "127.0.0.1:$CONTROL_PORT" \
        --identity "$WORK/operator" \
        --pin "$GUEST_PIN" \
        -- "$@" >"$WORK/ask-out" 2>"$WORK/ask-err"
    ASK_RC=$?
    set -e
}

# The first connection can arrive before `rivet-netd` has given eth0 its
# address. `smoke.sh` absorbs that race and so does this, for failures to REACH
# the machine only — a refusal from the machine is an answer and is reported the
# first time it is given.
ATTEMPT=1
while :; do
    ask get status
    [ "$ASK_RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 20 ] && { cat "$WORK/ask-err" >&2; fail "the machine never answered \`get status\`"; }
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done
note "the machine answers \`get status\` (after $ATTEMPT attempt(s))"

# --- 6. QUESTION 1: a real signature, verified out of the store -------------

note "deploying $(basename "$RVT_V1") — transfer, then install by digest, with --enable"
MARK_GOOD=$(mark)
deploy "$RVT_V1" "$WORK/deploy-v1.log" --enable
echo "--- cargo rivet deploy, first artifact ---"
cat "$WORK/deploy-v1.log"
[ "$DEPLOY_RC" -eq 0 ] || fail "the first deploy exited $DEPLOY_RC. Its output is above."

grep -qF "$DIG_V1" "$WORK/deploy-v1.log" \
    || fail "the deploy never named the digest $DIG_V1 the packager printed"
grep -qF "published into the fleet repository" "$WORK/deploy-v1.log" \
    || fail "the deploy did not say it published into the fleet repository first"
grep -qF "digest  = \"$DIG_V1\"" "$DEPLOY_FLEET/manifest.toml" \
    || fail "the scratch fleet repository's manifest does not record $DIG_V1"
[ -f "$DEPLOY_FLEET/objects/$DIG_V1" ] \
    || fail "the scratch fleet repository holds no object named $DIG_V1"

# **The signature, checked out of the store.** The supervisor's own record names
# the object it resolved and handed to the installer, and that path is under the
# store's objects directory — which is the whole claim.
await "$MARK_GOOD" "$OBJECTS_DIR/$DIG_V1" 120 \
    || { since "$MARK_GOOD" | grep -a 'rivet-deploy' | tail -30 >&2
         fail "nothing on the console names $OBJECTS_DIR/$DIG_V1, so it cannot be said that the installer read the artifact OUT OF THE STORE. The console is $CONSOLE."; }
note "the installer was handed $OBJECTS_DIR/$DIG_V1"

# The installer's own records. `ADR-022` §3.2 — the guest saying what it did,
# rather than the client saying what it asked for.
await "$MARK_GOOD" 'the application is installed' 240 \
    || { since "$MARK_GOOD" | grep -a 'rivet-deploy' | tail -20 >&2
         fail "cmd/rivet-deploy never recorded an installed application. The console is $CONSOLE." ; }
echo "--- what the installer recorded ---"
since "$MARK_GOOD" | grep -ao '"msg":"the payload.s header is signed by a key[^"]*"' | head -1 || true
since "$MARK_GOOD" | grep -ao '"msg":"every member[^"]*"' | head -1 || true
since "$MARK_GOOD" | grep -ao '"msg":"the application is installed[^"]*"' | head -1

# **And ADOPTED.** Installing does not start it (`ADR-015` §3.2.2); `--enable`
# put `enabled = true` in the document, and the supervisor's next convergence is
# what starts the program. Its own stdout is the evidence — captured by the
# supervisor and re-emitted as a journal record, so it arrives inside a `msg`
# field and an anchored grep would find nothing (`adr-042-proof.sh` §the probe's
# own lines).
await "$MARK_GOOD" 'hello-rivet: tick' 240 \
    || { since "$MARK_GOOD" | grep -a 'hello-rivet' | tail -30 >&2
         fail "the application was installed and the supervisor never ran it. What the console says about it is above."; }
echo "--- the application's own words, on the guest ---"
since "$MARK_GOOD" | grep -ao 'hello-rivet: tick [0-9]*' | head -3

# --- 7. QUESTION 2: an artifact this machine will not verify ----------------
#
# **Refused AT THE DOOR, since 2026-09-12** (`ADR-052` §7.2.3). Until that day
# this section watched the untrusted artifact reach the store and the INSTALLER
# refuse it — and the refused `0.9.9` then sat in the store holding one of
# hello-rivet's three retention slots, which is the defect the maintainer's
# ruling removed. Now the bulk endpoint verifies before it names anything, so
# what must be seen is: the deploy says the machine refused the ARTIFACT (not
# that the transfer failed), the machine's own log says why, the digest is in
# neither `objects/` nor `incoming/`, and no install was ever asked for.

note "deploying $(basename "$RVT_BAD") — signed by a key this machine does not trust"
MARK_BAD=$(mark)
DEPLOY_SEED=$UNTRUSTED_KEY
DEPLOY_FLEET=$WORK/fleet-untrusted
deploy "$RVT_BAD" "$WORK/deploy-bad.log"
DEPLOY_SEED=
DEPLOY_FLEET=$WORK/fleet
echo "--- cargo rivet deploy, the untrusted artifact ---"
cat "$WORK/deploy-bad.log"

[ "$DEPLOY_RC" -ne 0 ] \
    || fail "cargo rivet deploy exited 0 for an artifact signed by a key the machine does not trust. A refusal reported as a success is this proof's worst outcome."

# **The client's words: the ARTIFACT was refused and the transfer was intact.**
# Reporting this as a transfer failure is the mirror image of the defect of
# 2026-09-12, and an operator reading it would retry something that will be
# refused identically.
grep -qF "REFUSED THE ARTIFACT" "$WORK/deploy-bad.log" \
    || fail "the deploy did not say the machine REFUSED THE ARTIFACT. Its output is above."
grep -qF "not a transfer failure" "$WORK/deploy-bad.log" \
    || fail "the deploy did not say the transfer itself was intact"
if grep -qF "the transfer failed" "$WORK/deploy-bad.log"; then
    fail "the deploy reported a TRANSFER failure for an artifact the machine refused on its signature"
fi
# **And WHY**, in the verifier's own words — the signature, not the wire.
grep -qF "not in this machine's trust set" "$WORK/deploy-bad.log" \
    || fail "the refusal does not say the signing key is untrusted. Its output is above."
# The digest the machine hashed is the one the packager printed, which is what
# makes this a refusal of THESE bytes rather than of a corrupted copy of them.
grep -qF "$DIG_BAD" "$WORK/deploy-bad.log" \
    || fail "the refusal does not name the digest $DIG_BAD, so it cannot be said the transfer was intact"

# **The machine's own record of refusing it.** `cmd/rivet-mcpd` writes the
# reason when its door refuses, so the question can be answered from the board
# and not only from the client that was told.
await "$MARK_BAD" 'REFUSED AT THE DOOR' 60 \
    || { since "$MARK_BAD" | grep -a 'rivet-mcpd' | tail -20 >&2
         fail "rivet-mcpd never recorded a refusal at the door. The console is $CONSOLE."; }
echo "--- what the machine recorded ---"
since "$MARK_BAD" | grep -ao 'REFUSED AT THE DOOR[^"]*' | head -1

# **No install was asked for, so the installer never saw it.** The deploy stops
# at a refused push; the supervisor's record of a request would name the
# object's path, and the installer's refusal would follow it.
if since "$MARK_BAD" | grep -aqF "$OBJECTS_DIR/$DIG_BAD"; then
    since "$MARK_BAD" | grep -aF "$OBJECTS_DIR/$DIG_BAD" | tail -3 >&2
    fail "the untrusted object's store path appears on the console — something resolved a digest the door refused"
fi
if since "$MARK_BAD" | grep -aq 'the payload is refused'; then
    fail "the installer ran after the door refused — an install was requested for an artifact that should not exist"
fi

# **Never in the store: not in `objects/`, not left in `incoming/`.** Read off
# the machine with `get file`, never inferred from what the client printed.
ask get file "$OBJECTS_DIR"
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/ask-err" >&2; fail "\`get file $OBJECTS_DIR\` exited $ASK_RC"; }
if grep -qF "$DIG_BAD" "$WORK/ask-out"; then
    cat "$WORK/ask-out" >&2
    fail "$OBJECTS_DIR holds $DIG_BAD — the refused artifact was STORED"
fi
INCOMING_DIR=$ARTIFACTS_DIR/incoming
ask get file "$INCOMING_DIR"
# `incoming/` may legitimately be empty or absent; either way it must not hold
# the refused digest, or any object at all once every transfer has finished.
if [ "$ASK_RC" -eq 0 ] && grep -qE '[0-9a-f]{64}' "$WORK/ask-out"; then
    cat "$WORK/ask-out" >&2
    fail "$INCOMING_DIR holds an object after the refusal — a refused transfer left its bytes behind"
fi

# A second, independent reading, because the ones above are sentences and
# sentences get reworded: the machine's own `/state/app/hello-rivet` holds a
# directory per installed version (`rivet_core::package::install`'s `Plan`:
# `/state/app/<name>/<version>`, with `current` a symlink into one of them), and
# the untrusted artifact's 0.9.9 must not be among them.
#
# **NOT the installed `manifest.toml`**, which was the first thing tried and
# says nothing: `cargo rivet package` renders a manifest from
# `[package.metadata.rivet]` and a version is not one of its keys — the version
# is in the SIGNED HEADER. A check on that file would have passed whatever was
# installed.
ask get file "/state/app/hello-rivet"
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/ask-err" >&2; fail "could not list /state/app/hello-rivet"; }
echo "--- what /state/app/hello-rivet holds ---"
cat "$WORK/ask-out"
if grep -q '0\.9\.9' "$WORK/ask-out"; then
    fail "/state/app/hello-rivet has a 0.9.9 directory — the artifact signed by an untrusted key WAS unpacked"
fi
grep -q "$V1" "$WORK/ask-out" \
    || fail "/state/app/hello-rivet holds neither $V1 nor 0.9.9, so this check cannot tell which artifact is installed"
note "the untrusted artifact was refused AT THE DOOR: never stored, never installed, and the deploy said why"

# --- 7b. an artifact for ANOTHER ARCHITECTURE ------------------------------
#
# `PLAN-Feature-Set-2026-09-12` §3.3g. Correctly signed by a key this machine
# trusts, intact, and built for `armv6` — until 2026-09-12 the door admitted it
# and the installer unpacked it, because `Application::accepts` had no caller.
# The guest's image records `aarch64` (`run.sh` places the record), so this must
# be refused at the door exactly as the untrusted one was, with both
# architectures named.

note "deploying $(basename "$RVT_ARCH") — trusted, intact, and built for armv6"
MARK_ARCH=$(mark)
deploy "$RVT_ARCH" "$WORK/deploy-armv6.log"
echo "--- cargo rivet deploy, the armv6 artifact ---"
cat "$WORK/deploy-armv6.log"

[ "$DEPLOY_RC" -ne 0 ] \
    || fail "cargo rivet deploy exited 0 for an armv6 artifact on an aarch64 machine"
grep -qF "REFUSED THE ARTIFACT" "$WORK/deploy-armv6.log" \
    || fail "the deploy did not say the machine REFUSED THE ARTIFACT. Its output is above."
grep -qF "armv6" "$WORK/deploy-armv6.log" \
    || fail "the refusal does not name the artifact's architecture"
grep -qF 'this machine executes "aarch64"' "$WORK/deploy-armv6.log" \
    || fail "the refusal does not name this machine's architecture as its image records it"
if grep -qF "not in this machine's trust set" "$WORK/deploy-armv6.log"; then
    fail "the armv6 artifact was refused on its SIGNATURE, so this run says nothing about architecture — it must be sealed with the key the guest trusts"
fi
await "$MARK_ARCH" 'REFUSED AT THE DOOR' 60 \
    || { since "$MARK_ARCH" | grep -a 'rivet-mcpd' | tail -20 >&2
         fail "rivet-mcpd never recorded refusing the armv6 artifact at the door. The console is $CONSOLE."; }
if since "$MARK_ARCH" | grep -aqF "$OBJECTS_DIR/$DIG_ARCH"; then
    fail "the armv6 object's store path appears on the console — something resolved a digest the door refused"
fi
ask get file "$OBJECTS_DIR"
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/ask-err" >&2; fail "\`get file $OBJECTS_DIR\` exited $ASK_RC"; }
if grep -qF "$DIG_ARCH" "$WORK/ask-out"; then
    cat "$WORK/ask-out" >&2
    fail "$OBJECTS_DIR holds $DIG_ARCH — the armv6 artifact was STORED"
fi
ask get file "/state/app/hello-rivet"
if grep -q '0\.1\.5' "$WORK/ask-out"; then
    fail "/state/app/hello-rivet has a 0.1.5 directory — the armv6 artifact WAS unpacked"
fi
note "the armv6 artifact was refused AT THE DOOR on its architecture: never stored, never installed"

# --- 8. QUESTIONS 3 and 4: retention, on a filesystem in the guest ----------
#
# One object of another product first, so that "another product untouched" is a
# statement about something that was there before the prunes started.
note "deploying $(basename "$RVT_OTHER") — a second product, one version"
deploy_and_settle "$RVT_OTHER" "$WORK/deploy-other.log"
echo "--- cargo rivet deploy, the second product ---"
cat "$WORK/deploy-other.log"
[ "$DEPLOY_RC" -eq 0 ] || fail "the second product's deploy exited $DEPLOY_RC"

for _pair in "$V2:$RVT_V2" "$V3:$RVT_V3" "$V4:$RVT_V4"; do
    _v=${_pair%%:*}
    _rvt=${_pair#*:}
    note "deploying $(basename "$_rvt")"
    deploy_and_settle "$_rvt" "$WORK/deploy-$_v.log"
    echo "--- cargo rivet deploy, hello-rivet $_v ---"
    cat "$WORK/deploy-$_v.log"
    [ "$DEPLOY_RC" -eq 0 ] || fail "the deploy of hello-rivet $_v exited $DEPLOY_RC"
done

# **The fourth arrival's install was asked for and answered.** That is
# `b7d1d943`'s defect, exercised: a prune that dropped the arriving object would
# leave this install naming a digest the store no longer held, and the deploy
# above would have failed rather than exited 0.
grep -qF "$DIG_V4" "$WORK/deploy-$V4.log" \
    || fail "the fourth deploy never named $DIG_V4"

# **The store, read off the machine.** `get file` on a directory answers with
# its entries; the entries here are digests, and each one holds exactly one
# file. Nothing about this is inferred from what the client printed.
ask get file "$OBJECTS_DIR"
[ "$ASK_RC" -eq 0 ] || { cat "$WORK/ask-err" >&2; fail "\`get file $OBJECTS_DIR\` exited $ASK_RC"; }
echo "--- the store, as the machine lists it ---"
cat "$WORK/ask-out"
HELD=$(grep -oE '[0-9a-f]{64}' "$WORK/ask-out" | sort -u)
[ -n "$HELD" ] || fail "the machine listed no objects at all in $OBJECTS_DIR"

# Name each one, by asking the machine what is inside its directory.
NAMES=$WORK/held-names
: >"$NAMES"
for _d in $HELD; do
    ask get file "$OBJECTS_DIR/$_d"
    [ "$ASK_RC" -eq 0 ] || fail "\`get file $OBJECTS_DIR/$_d\` exited $ASK_RC"
    _name=$(grep -oE '[A-Za-z0-9._+-]+\.rvt' "$WORK/ask-out" | head -1)
    [ -n "$_name" ] || { cat "$WORK/ask-out" >&2; fail "the object $_d holds no .rvt"; }
    printf '%s %s\n' "$_d" "$_name" >>"$NAMES"
done
echo "--- what the store holds ---"
sed 's/^/  /' "$NAMES"

_hello=$(grep -c ' hello-rivet-' "$NAMES" || true)
_other=$(grep -c ' slow-stop-probe-' "$NAMES" || true)
[ "$_hello" -eq "$KEEP" ] \
    || fail "the store holds $_hello hello-rivet objects and rivet_core::artifact::KEEP is $KEEP. Four arrived (the untrusted one was refused at the door); retention should have left $KEEP."
[ "$_other" -eq 1 ] \
    || fail "the store holds $_other slow-stop-probe objects and exactly one arrived. Retention touched another product's group."
grep -q "^$DIG_V4 " "$NAMES" \
    || fail "the artifact that arrived LAST is not in the store. That is the defect b7d1d943 fixed by unit test, reproducing on a machine."
note "retention: $KEEP of hello-rivet, 1 of slow-stop-probe, and the newest arrival survived its own prune"

# **The evictions are counted from what the store now holds**, and NOT from what
# the client printed — because the client prints nothing about them.
#
# `cmd/rivet-mcpd`'s bulk endpoint composes a `pruned …` line into the 201's
# body naming every artifact it dropped, and `rivet_remote::tls::Remote::push`
# reads that answer only to check the status line and then discards it. So the
# one place a Rivet says which artifacts it deleted is thrown away by the tool
# that caused the deletion. Recorded here rather than worked around: this
# script asks the machine directly, which is better evidence anyway, and the
# gap is a finding rather than a thing to assert on.
_arrived_hello=4
_evicted=$((_arrived_hello - _hello))
[ "$_evicted" -ge 1 ] \
    || fail "four hello-rivet artifacts arrived and $_hello are held, so nothing was ever evicted and the retention rule did not run"
note "$_arrived_hello hello-rivet artifacts arrived, $_hello are held — $_evicted evicted"
grep -q "^$DIG_V4 " "$NAMES" \
    || fail "the last arrival is not held, so it was evicted by its own prune"

# **Which three, and that the refused artifact holds none of them.** Until the
# door existed the untrusted 0.9.9 was stored, and — as the highest version — it
# kept one of hello-rivet's three slots, so this run held 0.1.3, 0.1.4 and 0.9.9.
# That was the defect. With nothing unverified admitted, the three highest
# TRUSTED versions are what is held.
for _pair in "$V2:$DIG_V2" "$V3:$DIG_V3" "$V4:$DIG_V4"; do
    grep -q "^${_pair#*:} " "$NAMES" \
        || fail "hello-rivet ${_pair%%:*} is not held, and it is one of the three highest versions that arrived"
done
if grep -q "^$DIG_BAD " "$NAMES"; then
    fail "the untrusted 0.9.9 is in the store and holding a retention slot — the door let it in"
fi
note "held: hello-rivet $V2, $V3 and $V4 — the refused 0.9.9 holds no slot"

# --- 8b. what this machine is ALREADY RUNNING -------------------------------
#
# `PLAN-Feature-Set-2026-09-12` §3.3f, found on `rivet-01`: re-sending the build
# a machine runs requested the install again, and the installer wrote a second
# copy of the running build over the rollback. `current` selects hello-rivet
# $V4 now, so sending $V4 again must transfer, then STOP: the deploy exits 0,
# says the machine already runs it and that no install was requested, and the
# installer never runs.
#
# **What this can and cannot show.** Every artifact here is an application, so
# this is the `current` half. The system-update half — the running SLOT's
# retained header — needs a slot, which QEMU does not have (`ADR-026` §4).
note "deploying $(basename "$RVT_V4") AGAIN — the version current already selects"
MARK_RUNNING=$(mark)
deploy "$RVT_V4" "$WORK/deploy-again.log"
echo "--- cargo rivet deploy, the running version again ---"
cat "$WORK/deploy-again.log"
[ "$DEPLOY_RC" -eq 0 ] \
    || fail "re-sending the running version exited $DEPLOY_RC. The machine is in the state the deploy was for, which is not a failure."
grep -qF "ALREADY RUNNING" "$WORK/deploy-again.log" \
    || fail "the deploy did not say the machine is already running this artifact"
grep -qF "NO INSTALL WAS REQUESTED" "$WORK/deploy-again.log" \
    || fail "the deploy did not say no install was requested"
if grep -qF "install  requested" "$WORK/deploy-again.log"; then
    fail "the deploy reported an install request for the version the machine already runs"
fi
grep -qF "sent     " "$WORK/deploy-again.log" \
    || fail "the running version was not transferred first — the store must still be offered it"
# **The machine's side**: no installer ran. Given a generous window, because the
# claim is an absence and a short one would prove only impatience.
sleep 15
if since "$MARK_RUNNING" | grep -aqE 'the application is installed|nothing is ever unpacked'; then
    since "$MARK_RUNNING" | grep -a 'rivet-deploy' | tail -10 >&2
    fail "an installer ran for the version the machine already runs"
fi
ask get install
echo "--- get install, after the re-send ---"
cat "$WORK/ask-out"
note "re-sending the running version transferred it and stopped: no install was requested and none ran"

# --- 9. what this run settled ----------------------------------------------

echo
echo "adr-052-proof: PASS"
echo
echo "  What ran, on $RIVET_KERNEL under QEMU -M virt:"
echo "    * seven real .rvt files from \`cargo rivet package\`, six sealed with the"
echo "      application seed and one with a key the guest does not trust;"
echo "    * each transferred over ADR-035's bulk endpoint, whose door VERIFIED it"
echo "      against the machine's trust store before naming it in the store;"
echo "    * \`rivet apply install <digest> confirm\` for each admitted one, over"
echo "      mutual TLS;"
echo "    * cmd/rivet-deploy, started by the supervisor from the request, opening"
echo "      the object the store resolved and VERIFYING its signature again against"
echo "      keys/rivet-app.pub in the machine's trust store;"
echo "    * the application installed, enabled in the document, and started — its"
echo "      own stdout on the console;"
echo "    * the artifact signed by an untrusted key REFUSED AT THE DOOR: the deploy"
echo "      said the artifact was refused and why, and it was never stored, never"
echo "      left in incoming/, and never installed;"
echo "    * a TRUSTED artifact whose header says armv6 REFUSED AT THE DOOR against"
echo "      the aarch64 the guest's architecture record states — never stored or"
echo "      installed;"
echo "    * retention leaving the three highest trusted versions of one product and"
echo "      1 of another, with the newest arrival among the survivors;"
echo "    * the version \`current\` selects, sent AGAIN: transferred, then the deploy"
echo "      stopped with exit 0 saying the machine already runs it, and no"
echo "      installer ran."
echo
echo "  What it does NOT settle:"
echo "    * /state here is the initramfs tmpfs. --card gives a guest real ext4 and"
echo "      refuses --document and --secrets with it, so a machine with a card"
echo "      cannot receive a transfer and this proof cannot have both."
echo "    * ADR-026 §4's right-hand column entirely: SD timing, the firmware, the"
echo "      VideoCore boot chain. QEMU is not a board."
echo "    * nothing about a system update. Every artifact here is an application,"
echo "      so neither the running SLOT's retained header nor the refusal during"
echo "      an uncommitted trial is exercised: QEMU has no slots."
echo
echo "  The console is $CONSOLE, and it OUTLIVES this script."
