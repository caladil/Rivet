#!/bin/sh
# tools/qemu/adr-042-net-proof.sh — **LANDLOCK_ACCESS_NET_BIND_TCP is handled
# and granted to nothing, and nothing that ought to start stops starting.**
#
# `ADR-042` §7's amendment. Run it in the container:
#
#   RIVET_CACHE_TAG=-netlock tools/container/exec.sh tools/qemu/adr-042-net-proof.sh
#
# ## Why this is a second script and not a stage bolted onto adr-042-proof.sh
#
# `tools/qemu/adr-042-proof.sh` proves the filesystem half of `ADR-042` with a
# single-boot, no-control-plane harness: a console log is enough when the only
# question is which paths a confined process could open. The question here
# needs `rivet get status` over the real control plane — the failure mode that
# matters is not an insecure board, it is a board where an application will
# not run, and a console log alone cannot show a service's restart count or
# whether the supervisor still holds its pid. That needs `rivet-mcpd`,
# `rivet-netd`, the `rivet` CLI packed at `--cli` (`rivet-mcpd` hands every
# connection the stdin and stdout of `rivet mcp`, and without the binary
# packed the handshake succeeds and every request then dies with `ENOENT` —
# measured, not assumed, the first time this script ran without it), two
# throwaway identities and a document with a `[control-plane]` table — the
# shape `adr-045-proof.sh` and `adr-051-proof.sh` already use, not
# `adr-042-proof.sh`'s.
#
# ## The two halves, and why one alone is worthless
#
# 1. **A confined application that tries to `bind` a TCP port is refused.**
#    `tools/qemu/testapp`'s `confined` mode now also tries a loopback bind and
#    prints whatever the kernel said — `AGENTS.md` §7: read off the running
#    guest, not asserted from documentation.
# 2. **Every service this harness equips still starts.** The ruleset is now
#    built from a THREE-field struct on this kernel instead of a bare `u64`,
#    and a mistake in that construction is a `landlock_create_ruleset` that
#    fails for every confined application, on every boot — not a narrower
#    sandbox, a broken one. So this asserts the shape `adr-045-proof.sh` does
#    for the confined application: `state` is `started`, `restarts` is `0`,
#    and the supervisor still holds a `pid`. `rivet-mcpd` gets the same
#    assertion, as the unconfined control — `ADR-042` applies to applications
#    only (`cmd/rivet-sysd/src/applications.rs` is the only call site that
#    ever sets `Program.confinement`; every system service definition in
#    `cmd/rivet-sysd/src/definitions.rs` carries `confinement: None`), so
#    neither it nor `rivet-netd` is expected to be touched by this change at
#    all. `rivet-netd` is checked too, but not for `started`: it converges
#    `eth0` and exits `0` on purpose, so its assertion is that it is present
#    and not `failed`, not that it is still running.
#
# ## What it does NOT settle
#
# `ADR-026` §4's right-hand column, entirely: no Raspberry Pi, no SD card, no
# real timing. And it says nothing about `LANDLOCK_ACCESS_NET_CONNECT_TCP`,
# which `ADR-042` §7 argues is a separate decision this change does not make.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO/target` — `tools/container/exec.sh` mounts a
# named volume there and exports it, so a script that composed the in-tree path
# would build correctly and then look where nothing put the binaries.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
OUT=$TARGET_DIR/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-042-net-proof
rm -rf "$WORK"
mkdir -p "$WORK"
chmod 700 "$WORK"

VERDICT=0
fail() {
    echo "adr-042-net-proof: FAIL — $1" >&2
    VERDICT=1
}
die() {
    echo "adr-042-net-proof: $1" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Constants read out of the source, never written here — `adr-045-proof.sh`'s
# own argument: a harness that restated one would pack a file where nothing
# looks for it the day a ruling moves it.
# ---------------------------------------------------------------------------
constant() {
    file=$1
    name=$2
    value=$(sed -n "s/^pub const $name: *&str *= *\"\([^\"]*\)\".*/\1/p" "$file" | head -1)
    [ -n "$value" ] || die "no $name in $file — refusing to guess it"
    printf '%s\n' "$value"
}
number() {
    file=$1
    name=$2
    value=$(sed -n "s/^pub const $name: *u\(16\|64\) *= *\([0-9_]*\).*/\2/p" "$file" \
        | tr -d '_' | head -1)
    [ -n "$value" ] || die "no $name in $file — refusing to guess it"
    printf '%s\n' "$value"
}

DEFINITIONS_RS=$REPO/cmd/rivet-sysd/src/definitions.rs
CONTROL_PLANE_RS=$REPO/rivet-config/src/control_plane.rs
MCPD_PEERS_PATH=$(constant "$DEFINITIONS_RS" MCPD_PEERS_PATH)
CONTROL_PORT=$(number "$CONTROL_PLANE_RS" DEFAULT_PORT)

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
[ -x "$CC_aarch64_unknown_linux_musl" ] || die "no cross C compiler at \
$CC_aarch64_unknown_linux_musl — rivetos-mcpd links ring, whose build compiles C \
(ADR-009 §3.7.4). Point RIVET_BR_OUTPUT at the Buildroot tree."

echo "adr-042-net-proof: building the guest's binaries for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-netd -p rivetos-mcpd ) \
    || die "the guest's binaries did not build"

echo "adr-042-net-proof: building the confinement probe"
( cd "$HERE/testapp" && cargo build --release --target "$TARGET" ) \
    || die "tools/qemu/testapp did not build"
PROBE=$OUT/rivet-testapp
[ -f "$PROBE" ] || die "the probe was not built at $PROBE"

echo "adr-042-net-proof: building cargo-rivet for the HOST — the control-plane client"
( cd "$REPO" && cargo build --release -p cargo-rivet ) \
    || die "cargo-rivet did not build; without it this proof cannot ask the guest anything"
# **The repository's own, by path.** The `cargo-rivet` on PATH inside the
# container is stale and refuses manifest keys this build writes.
CARGO_RIVET=$TARGET_DIR/release/cargo-rivet

( cd "$HERE/fixture-signer" && cargo build --release ) || die "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer

# ---------------------------------------------------------------------------
# The confined application — the SAME fixture and manifest adr-042-proof.sh
# uses, installed under the name `tools/qemu/testapp` picks the `confined`
# behaviour from. **Note what the manifest does NOT declare: a `listen`.**
# Nothing about this proof's bind attempt is arbitrated by the supervisor —
# that is the whole point, an application binding its own port is doing
# something no manifest declared.
# ---------------------------------------------------------------------------
NAME=confinement-probe
INSTALLED=$WORK/$NAME
cp "$PROBE" "$INSTALLED"

HEADER=$WORK/$NAME.header
TRUST=$WORK/$NAME.trust
"$SIGNER" "$NAME" "$INSTALLED" "$HERE/confinement-probe.manifest.toml" \
    "$HEADER" "$TRUST" >/dev/null \
    || die "the confinement probe could not be signed"

# ---------------------------------------------------------------------------
# Two throwaway identities, minted per run — `AGENTS.md` §8.3 keeps key
# material out of tracked files, so `cargo rivet identity new` mints a fresh
# pair into this run's own `$WORK` and nothing here is committed.
# ---------------------------------------------------------------------------
echo "adr-042-net-proof: minting THROWAWAY identities in $WORK"
"$CARGO_RIVET" identity new --directory "$WORK/guest" --name adr042net-guest \
    >"$WORK/mint-guest" 2>&1 || { cat "$WORK/mint-guest" >&2; die "no guest identity"; }
"$CARGO_RIVET" identity new --directory "$WORK/operator" --name adr042net-operator \
    >"$WORK/mint-operator" 2>&1 || { cat "$WORK/mint-operator" >&2; die "no operator identity"; }
GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/guest" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 \
    | grep -oE '[0-9a-f]{64}' | head -1)
[ -n "$GUEST_PIN" ] || die "could not read the guest's public key back out of its certificate"
[ -n "$OPERATOR_PIN" ] || die "could not read the operator's public key"
[ "$GUEST_PIN" != "$OPERATOR_PIN" ] \
    || die "the two identities minted the same key, so nothing below distinguishes the ends"

mkdir -p "$WORK/secrets"
cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/" \
    || die "could not assemble the guest's identity directory"
printf '# one hex public key per line\n%s\n' "$OPERATOR_PIN" \
    >"$WORK/secrets/${MCPD_PEERS_PATH##*/}"

# ---------------------------------------------------------------------------
# The document
# ---------------------------------------------------------------------------
DOC=$WORK/machine.toml
{
    printf 'version = 1\n'
    printf 'hostname = "adr042net-guest"\n'
    sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$HERE/qemu-network.toml"
    printf '\n[services.%s]\nenabled = true\n' "$NAME"
    printf '\n[applications.%s]\ntheme = "digital-rain"\n' "$NAME"
    printf '\n[control-plane]\n'
    printf 'listen = "0.0.0.0:%s"\n' "$CONTROL_PORT"
} >"$DOC"
[ "$(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC")" -eq 1 ] \
    || { cat -n "$DOC" >&2; die "the composed document has more than one \`version\` key"; }

# ---------------------------------------------------------------------------
# Boot
# ---------------------------------------------------------------------------
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

LOG=$WORK/console.log
echo "adr-042-net-proof: booting with the confinement probe installed, control plane on $CONTROL_PORT"
"$HERE/run.sh" "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --cli "$OUT/rivet" \
    --netd "$OUT/rivet-netd" \
    --mcpd "$OUT/rivet-mcpd" \
    --secrets "$WORK/secrets" \
    --document "$DOC" \
    --app "$NAME:$INSTALLED:$HERE/confinement-probe.manifest.toml:$HEADER" \
    --trust "$TRUST" \
    --font "$REPO/assets/fonts/Geist.ttf" \
    --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
    >"$LOG" 2>&1 </dev/null &
QEMU=$!
# shellcheck disable=SC2064
trap "kill $QEMU 2>/dev/null || true" EXIT

# Wait for the DAEMON's own line rather than for a number of seconds: it is
# the earliest moment a connection could be accepted.
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
echo "adr-042-net-proof: the control plane is up after ${WAITED}00ms"

# ---------------------------------------------------------------------------
# Half 1 — the bind is refused, with the kernel's own errno
# ---------------------------------------------------------------------------
echo
echo "--- half 1: a confined TCP bind is refused ---"
# **Not anchored at the start of a line** — `adr-042-proof.sh`'s own note:
# the supervisor captures an application's stdout (`ADR-022` §4) and
# re-emits each line as a journal record, so the probe's words arrive inside
# a JSON `msg` field rather than on their own.
BIND_LINE=$(grep -ao 'confined: BIND_TCP[^"]*' "$LOG" | head -1)
if [ -z "$BIND_LINE" ]; then
    echo "adr-042-net-proof: THE PROBE NEVER TRIED A BIND. The whole console follows." >&2
    cat "$LOG" >&2
    die "no \`confined: BIND_TCP\` line on the console"
fi
echo "  the probe said: $BIND_LINE"
case $BIND_LINE in
    *'refused,'*)
        echo "  OK   the bind was refused"
        ;;
    *)
        fail "the confined probe bound a TCP port — THE RULESET DID NOT REFUSE IT: $BIND_LINE"
        ;;
esac

# **The ABI, read off the same console the probe's own line came from** —
# `AGENTS.md` §7: a refusal is evidence of ADR-042 §7 only on a kernel that
# actually has the right to refuse with. Below ABI 4 a refused bind here
# would be nothing more than this kernel's ordinary uid-based permission
# model, and the note says so instead of the run silently proving less than
# it appears to.
ABI_LINE=$(grep -a 'landlock_abi' "$LOG" | head -1)
echo "  the supervisor said: ${ABI_LINE:-(nothing)}"
ABI=$(printf '%s' "$ABI_LINE" | grep -oE '"landlock_abi":[0-9]+' | grep -oE '[0-9]+' | head -1)
if [ -n "$ABI" ] && [ "$ABI" -ge 4 ]; then
    echo "  OK   landlock_abi $ABI is >= 4, so BIND_TCP is a right this kernel actually has"
else
    echo "  NOTE landlock_abi is ${ABI:-unknown}; below 4 the refusal above proves nothing about" >&2
    echo "       ADR-042 §7 (AGENTS.md §7 — a claim of enforcement needs the same checking a" >&2
    echo "       claim of absence does)" >&2
fi

# ---------------------------------------------------------------------------
# Half 2 — everything that ought to start still starts
# ---------------------------------------------------------------------------
echo
echo "--- half 2: nothing that ought to start stops starting ---"
# **Retried on a connection failure only, `adr-045-proof.sh`'s own pattern.**
# `rivet-mcpd` binds before every service has necessarily finished its first
# convergence pass, so a `get status` thrown at the control plane the instant
# it reports listening can land on a half-settled machine. A refusal FROM the
# machine is an answer and is reported first time; only a connection that
# never completed is retried.
ask_status() {
    set +e
    "$CARGO_RIVET" run \
        --host "127.0.0.1:$CONTROL_PORT" \
        --identity "$WORK/operator" \
        --pin "$GUEST_PIN" \
        -- get status \
        >"$WORK/status" 2>"$WORK/status-err"
    RC=$?
    set -e
}
ATTEMPT=1
while :; do
    ask_status
    [ "$RC" -eq 0 ] && break
    [ "$ATTEMPT" -ge 6 ] && break
    grep -qiE 'connection (refused|reset)|timed out|os error|0 byte\(s\) arrived' \
        "$WORK/status-err" || break
    echo "adr-042-net-proof: \`get status\` did not complete yet (attempt $ATTEMPT) — retrying"
    ATTEMPT=$((ATTEMPT + 1))
    sleep 2
done
[ "$RC" -eq 0 ] || { cat "$WORK/status-err" >&2; die "\`rivet get status\` failed"; }

row_of() {
    tr -d '\n' <"$WORK/status" | grep -o "\"name\":\"$1\"[^}]*"
}
assert_started() {
    label=$1
    row=$(row_of "$1")
    if [ -z "$row" ]; then
        fail "$label does not appear in \`get status\` at all"
        return
    fi
    case $row in
        *'"state":"started"'*) echo "  OK   $label: state is \`started\`" ;;
        *) fail "$label: state is not \`started\`: $row" ;;
    esac
    case $row in
        *'"restarts":0'*) echo "  OK   $label: restarts is 0" ;;
        *) fail "$label: restarts is not 0, so it did not come up cleanly: $row" ;;
    esac
    case $row in
        *'"pid":'*) echo "  OK   $label: the supervisor holds a pid" ;;
        *) fail "$label: no pid — the supervisor is not holding it as running: $row" ;;
    esac
}

# **The confined application first.** This is the one a mistake in the new
# three-field `landlock_ruleset_attr` construction breaks first and worst: a
# `landlock_create_ruleset` that answers `EINVAL` refuses the spawn entirely
# (`ADR-042` §5's second gate, in `cmd/rivet-sysd/src/kernel.rs`), which
# reads here as `state` never reaching `started` at all rather than as a
# narrower sandbox.
assert_started "$NAME"

# **`rivet-mcpd` — the unconfined control that answered this very request.**
# Nothing in this change touches `cmd/rivet-sysd/src/definitions.rs`, so a
# system service is not expected to be affected at all; it is asserted rather
# than assumed, so that a defect that somehow reached the ABI-gate in
# `crate::kernel::spawn` for every process (not only confined ones) would be
# caught here instead of read as "the confined proof failed, so the system
# services are probably fine". Its `started` state is proven twice over: this
# assertion, and the fact that `get status` above answered at all.
assert_started "rivet-mcpd"

# **`rivet-netd` — a ONE-SHOT converge, not a persistent service, and its
# success looks like an exit rather than a `started` state.** It configures
# `eth0` and exits `0`; `restart = "on-failure"` means a clean exit does not
# restart it, so by the time this line runs it has already left `started` on
# purpose. `assert_started` would read that correct behaviour as a failure,
# which is why the check here is only that it converged and was not marked
# failed — not that it is still `started`.
NETD_ROW=$(row_of "rivet-netd")
if [ -z "$NETD_ROW" ]; then
    fail "rivet-netd does not appear in \`get status\` at all"
elif printf '%s' "$NETD_ROW" | grep -q '"state":"failed"'; then
    fail "rivet-netd is \`failed\`: $NETD_ROW"
else
    NETD_STATE=$(printf '%s' "$NETD_ROW" | grep -o '"state":"[a-z]*"' | sed 's/.*:"\([a-z]*\)"/\1/')
    echo "  OK   rivet-netd: present and not \`failed\` (state: $NETD_STATE — a one-shot converge \
is expected to have already exited)"
fi

echo
if [ "$VERDICT" -eq 0 ]; then
    echo "adr-042-net-proof: PASS — a confined bind was refused, and nothing stopped starting"
else
    echo "adr-042-net-proof: FAIL — the console log and the status answer are in $WORK" >&2
fi
exit "$VERDICT"
