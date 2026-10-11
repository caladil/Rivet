#!/bin/sh
# tools/qemu/smoke.sh — automated pass/fail boot test for CI and humans.
#
# run.sh (this directory) is interactive: a human watches the console and
# presses Ctrl-A x when done. This is the one that gives a yes/no answer
# without a human in the loop: build the workspace, boot the result under
# QEMU with a bounded timeout, assert on the console, and exit non-zero
# naming exactly what failed.
#
# See docs/ADR-026, especially §4 — a green run here means PID 1 semantics,
# the aarch64-unknown-linux-musl toolchain, and syscall behaviour against the
# real target kernel are fine. It settles NOTHING about the VideoCore boot
# chain, tryboot, bcmgenet or brcmfmac — ADR-026 §4's right-hand column is the
# board's alone. Passing here earns the right to try hardware; it is not a
# claim that the change works (AGENTS.md §6).
#
# AGENTS.md §3: stubs refuse, they never fake success. A test that reports
# success without actually running is worse than no test, so every missing
# prerequisite below (no QEMU, no kernel image, no binary) is a named, loud,
# non-zero failure — never a silent skip that looks like a pass.
#
# Usage:
#   ./smoke.sh                      builds rivet-initd, rivet-sysd, the rivet
#                                    CLI, rivet-timed, rivet-netd and
#                                    rivet-mcpd for aarch64-musl release, plus
#                                    cargo-rivet for the HOST, and makes BOTH
#                                    boots below.
#   ./smoke.sh <init>               skips the build and boots that binary as
#                                    PID 1 with NO supervisor packed. Fewer
#                                    assertions — see below. Used to prove this
#                                    harness actually catches a broken init,
#                                    and to reproduce PID 1's give-up path.
#   ./smoke.sh <init> <supervisor>   skips the build and boots that pair, with
#                                    NO CLI — which asserts the *negative*
#                                    direction of the login path below.
#   ./smoke.sh <init> <supervisor> <cli>
#                                    skips the build and boots all three.
#   ./smoke.sh <init> <supervisor> <cli> <mcpd>
#                                    ...and the control plane, which is what
#                                    makes the second boot happen.
#
# ## Two boots, and the second one is the one that ASKS
#
# **Boot 1** is the machine with nothing configured: no network, no document,
# no control plane. Everything below about PID 1, `/etc`, the journal, `/dev`
# and the boot verdict is asserted there, and so is one negative — with no
# `rivet-mcpd` packed the supervisor's spawn must REFUSE by name, which is what
# a card that does not carry it really does.
#
# **Boot 2** is the machine ANSWERING. It has a network, a document whose
# `[control-plane] listen` is not loopback, a throwaway TLS identity, a
# forwarded port, and `rivet-mcpd`. The assertion is `cargo rivet run … --
# get status` over `ADR-035`'s control plane: the guest's own catalogue
# operation, composed as an MCP tool call, carried over mutual TLS, answered by
# `rivet mcp` on the other end. Then `get catalogue`, then `apply power-off
# confirm` as the guest's last act.
#
# ## What the paired boot asserts about /etc, added 2026-08-23
#
# `/etc` has one generator (ADR-007 §3.5) and it is the supervisor's, running
# before it converges any service (ADR-017 §3.1.8). Before that existed, a
# running Rivet had no /etc at all — so the assertions below are the ones
# that would have failed on every boot up to that date, which is what makes
# them worth having rather than decorative. Each reads a value the supervisor
# read back OFF THE KERNEL — the file it wrote, and /proc/self/mounts — rather
# than one it merely intended.
#
# ## Why the pair is the default rather than a mode
#
# A boot with no supervisor is not the system; it is the system with its only
# child amputated. Every observation it can produce is of PID 1's *failure*
# path — retry, backoff, GIVING UP, boot confirmation withheld — because the
# binary PID 1 exists to spawn is not there. That is worth being able to
# reproduce, which is why the one-binary form is kept, but it is a narrower
# thing to assert on and a strange thing to test by default.
#
# So the argument count selects it: hand this script two binaries (or none, and
# it builds them) and it asserts the full set; hand it one and it asserts the
# four it always did, and names which run it made.
#
# ## What the third binary asserts about the login path, added 2026-08-23
#
# The projection's one runtime input is a *probe*: cmd/rivet-sysd's
# etc.rs asks the kernel whether anything is at
# each of rivet_core::etc::LOGIN_SHELL_PATHS, and writes an /etc/shells naming
# the ones that are there (measured on hardware — an account whose shell is
# unlisted is rejected *before* authentication). Until the CLI could be
# packed, that probe had exactly one reachable answer under this emulator, so
# every run reported "permits_a_login":false and the interaction between the
# image installing a CLI and the projection finding it was untestable here.
#
# Both answers are now reachable, and BOTH are asserted, in the run that
# produces each: with a CLI packed the console must name the shell, and without
# one it must say it names none. An assertion that can only ever pass is not a
# test (AGENTS.md §4), and the negative is a state a real card can be in rather
# than an artefact of the harness.
#
# **Nothing here execs the CLI.** PID 1 spawns the supervisor and nothing else,
# so what is settled is that the binary is FOUND at the path the projection
# probes — not that it runs, and not that a login works.
#
# ## What the control plane asserts, added 2026-08-30
#
# `ADR-017` §4.1 keeps a service's argument vector the image's, so the
# supervisor starts `rivet-mcpd` with `--document`, `--identity` and `--peers`
# and the program reads the address itself. Every one of those has to be true at
# once for a caller to reach it, and each of them has failed silently before:
#
# * the binary has to be **in the image** — `cmd/rivet-sysd`'s definition was
#   `enabled: false` for three whole phases and a card built from the defaults
#   booted with no control plane at all (`PLAN.md` §94.7);
# * the identity and the pin file have to be **on `/state`**, or the daemon
#   exits 2 before it binds;
# * `[control-plane] listen` has to be **something SLIRP can route to**. The
#   built-in default is `127.0.0.1:7777`, and a forwarded port cannot reach a
#   loopback listener.
#
# **Both directions are asserted, each in the boot that produces it** — with no
# `rivet-mcpd` packed the spawn must refuse and say so once; with everything
# packed the daemon must announce the address it took, from the table it read,
# and then answer. An assertion that can only ever pass is not a test
# (`AGENTS.md` §4).
#
# ## The identity is minted PER RUN, and never fixed
#
# Two of them: one for the guest, one for the caller, both by `cargo rivet
# identity new` into a private temporary directory that is removed on exit. The
# guest's public key becomes the client's `--pin`; the client's becomes the one
# line in the guest's `peers` file. `ADR-025` §2's trust is mutual and symmetric,
# and this is the smallest thing that exercises both halves of it.
#
# **A fixed pair in the repository was the alternative and is refused.**
# `AGENTS.md` §8.3 forbids key material in a tracked file, and the reason bites
# harder here than the rule states: a committed `device.seed` is a private key
# every reader of this repository holds, so any of them could impersonate any
# machine that pinned it — and the first person to copy the fixture onto a board
# because it was convenient would not be doing anything the tree told them not
# to. The cost of minting is two Ed25519 keypairs, which is microseconds beside
# a QEMU boot.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
TARGET_TRIPLE=aarch64-unknown-linux-musl
# **Where cargo actually puts things**, which is not always under the
# repository: `tools/container/Dockerfile` sets `CARGO_TARGET_DIR=/target` so
# that a container build does not fight the host's `target/` over the same
# directory with different toolchain paths inside it. This script used to
# compose `$REPO_ROOT/target/...` unconditionally, which in the container names
# a directory that does not exist — and the failure reads as *the build did not
# produce one*, naming a path nothing was ever going to write.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO_ROOT/target}
KERNEL=${RIVET_KERNEL:-$HERE/kernel8.img}
SUPERVISE_RS=$REPO_ROOT/cmd/rivet-initd/src/supervise.rs
CORE_ETC_RS=$REPO_ROOT/rivet-core/src/etc.rs
DEFINITIONS_RS=$REPO_ROOT/cmd/rivet-sysd/src/definitions.rs
CONTROL_PLANE_RS=$REPO_ROOT/rivet-config/src/control_plane.rs
LOG=$(mktemp)
trap 'rm -f "$LOG"' EXIT

fail() {
    echo >&2
    echo "smoke.sh: FAIL: $1" >&2
    if [ -s "$LOG" ]; then
        echo >&2
        echo "--- captured console output ---" >&2
        cat "$LOG" >&2
        echo "--- end captured console output ---" >&2
    fi
    exit 1
}

# Numbers and paths this harness must agree with a crate about are read out of
# that crate, never restated here. A copy would go stale silently and the
# failure would look like a broken boot rather than a stale test. The file is a
# parameter because there is more than one crate to agree with: PID 1 owns where
# the supervisor goes, and rivet-core owns where the projection looks for a
# login shell.
constant() {
    file=$1
    name=$2
    value=$(sed -n "s/^pub const $name: *[a-z0-9&]* *= *\"\{0,1\}\([^\";]*\)\"\{0,1\};.*/\1/p" \
        "$file" | head -1)
    if [ -z "$value" ]; then
        fail "cannot read $name out of $file — this harness has to agree with that crate about it and will not guess"
    fi
    # A constant that is itself read out of `rivet-core` — `MCPD_NAME` is
    # `rivet_core::service::MCPD_SERVICE_NAME` — is followed one step into that
    # module, rather than handing the path expression on as if it were the
    # value. Without this the script asserts about a service literally named
    # `rivet_core::service::MCPD_SERVICE_NAME`, finds no record for it, and
    # reports a boot defect that is a parsing one.
    case $value in
        rivet_core::*::*)
            module=${value#rivet_core::}
            module=${module%%::*}
            inner=${value##*::}
            source=$REPO_ROOT/rivet-core/src/$module.rs
            [ -f "$source" ] || source=$REPO_ROOT/rivet-core/src/$module/mod.rs
            [ -f "$source" ] \
                || fail "$name in $file is $value, and there is no rivet-core module $module to read it from"
            value=$(constant "$source" "$inner") || exit 1
            ;;
    esac
    printf '%s\n' "$value"
}

# --- prerequisites: fail loudly, never silently pass ---

command -v qemu-system-aarch64 >/dev/null 2>&1 \
    || fail "qemu-system-aarch64 not found on PATH — cannot boot anything. On Arch: pacman -S qemu-system-aarch64"
command -v timeout >/dev/null 2>&1 \
    || fail "coreutils 'timeout' not found on PATH — required to bound the QEMU run"
[ -f "$KERNEL" ] \
    || fail "no kernel image at $KERNEL — run ./fetch-kernel.sh <user@host> first, or set RIVET_KERNEL (docs/ADR-026 §6)"
[ -f "$SUPERVISE_RS" ] \
    || fail "cannot find $SUPERVISE_RS — this harness reads PID 1's own constants out of it"
[ -f "$CORE_ETC_RS" ] \
    || fail "cannot find $CORE_ETC_RS — this harness reads the /etc file list and the login-shell path out of the definition"
[ -f "$DEFINITIONS_RS" ] \
    || fail "cannot find $DEFINITIONS_RS — this harness reads the control plane's name and the paths its identity lives at out of the supervisor's own definitions"
[ -f "$CONTROL_PLANE_RS" ] \
    || fail "cannot find $CONTROL_PLANE_RS — this harness reads the control plane's default port out of the schema that owns it"

if [ $# -gt 4 ]; then
    fail "at most four binaries: <init> [<supervisor> [<cli> [<mcpd>]]]. Got $# arguments."
fi

SUP_BIN=
CLI_BIN=
# The control plane. Unlike the C mechanism it replaced, it IS cargo's, so the
# no-argument form builds it and the four-argument form names one.
MCPD_BIN=
# nushell's, and this script cannot build it either — it needs a C
# cross-toolchain despite being Rust (ADR-005 §5.6.3). `tools/nushell/build.sh`
# produces it and leaves it at $HOME/.rivet/nushell/bin/nu.
#
# **Environment only, deliberately: there is no fifth positional.** The four
# positionals are the binaries a run *must* choose between, and this is not one
# of them — a card without nu is a supported card (the ruling that makes nu optional), so
# the default run asserts the without-nu direction and the variable selects the
# other. Adding a positional would also mean four ways to arrive at a
# three-binary invocation, which is how the fourth one gets passed in the wrong
# slot.
NUSHELL_BIN=${RIVET_NUSHELL:-}
# Set only by the control-plane boot, which is the one that needs an address:
# a forwarded port reaches the guest's 10.0.2.15, and nothing gives the
# interface that address unless netd runs and converges the document's
# `[network]` table (ADR-017 §4.3). A machine without it boots perfectly and is
# reachable by nothing.
NETD_BIN=
# The workstation client. A HOST binary, not an aarch64 one — `cargo rivet run`
# is what an operator uses and is what `ADR-035` left as the way in, so it
# is what this harness uses too rather than a probe written here.
CARGO_RIVET=
# `rivet-timed` joins the default set on the same argument the CLI did: it is
# the ONLY service that emits ADR-022 §3 records rather than lines, so it is the
# only producer for §4's FIRST capture path — and a branch with no producer in
# this harness is a branch only a board can exercise. Added 2026-08-24.
TIMED_BIN=${RIVET_TIMED:-}
if [ $# -ge 1 ]; then
    INIT_BIN=$1
    if [ $# -ge 2 ]; then
        SUP_BIN=$2
    fi
    if [ $# -ge 3 ]; then
        CLI_BIN=$3
    fi
    if [ $# -ge 4 ]; then
        MCPD_BIN=$4
    fi
else
    command -v cargo >/dev/null 2>&1 \
        || fail "cargo not found on PATH — cannot build rivet-initd, rivet-sysd and rivet"
    INIT_BIN="$TARGET_DIR/$TARGET_TRIPLE/release/rivet-initd"
    SUP_BIN="$TARGET_DIR/$TARGET_TRIPLE/release/rivet-sysd"
    # The CLI joins the default set for the same reason the supervisor did: a
    # Rivet with no login shell is not the system either, and the run that
    # packs one is the only run in which the projection's probe can answer yes.
    # The negative is still reachable — it is what two arguments select.
    CLI_BIN="$TARGET_DIR/$TARGET_TRIPLE/release/rivet"
    # And rivet-timed, for the reason given where TIMED_BIN is declared: it
    # is the only producer of ADR-022 §4's first capture path.
    TIMED_BIN=${RIVET_TIMED:-$TARGET_DIR/$TARGET_TRIPLE/release/rivet-timed}
    # And the control plane and the interface bring-up, which are what the
    # second boot is made of. `rivet-netd` used to be built only by
    # a stage that no longer exists; it is unconditional now, because the boot
    # that needs it is.
    MCPD_BIN="$TARGET_DIR/$TARGET_TRIPLE/release/rivet-mcpd"
    NETD_BIN=${RIVET_NETD:-$TARGET_DIR/$TARGET_TRIPLE/release/rivet-netd}

    # **The cross C compiler, which this script needed none of until the
    # control plane joined the set.** `rivetos-mcpd` links `rivetos-tls`, which
    # links `ring`, whose build compiles C and assembly — so `cc-rs` has to be
    # told what to use. Without this it looks for `aarch64-linux-musl-gcc` on
    # PATH and the build fails inside `ring` with `error occurred in cc-rs:
    # failed to find tool`, which names neither Rivet nor the reason.
    #
    # `tools/card/build.sh` reads the same variable from the same tree and this
    # is deliberately the same two lines: `ADR-009` §3.7.4 priced this when
    # `ring` was adopted, and what it costs is one variable per build script.
    CROSS_CC="${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-2026.02.3}/host/bin/aarch64-linux-gcc"
    export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"
    if [ ! -x "$CC_aarch64_unknown_linux_musl" ]; then
        fail "no cross C compiler at $CC_aarch64_unknown_linux_musl.

    rivetos-mcpd links rivetos-tls, which links ring, whose build compiles C
    (ADR-009 §3.7.4). Buildroot has produced this compiler since Stage 0, beside
    the strip tools/card/build.sh uses. Point RIVET_BR_OUTPUT at that tree — in
    the container tools/container/qemu.sh sets it to the /br mount — or set
    CC_aarch64_unknown_linux_musl yourself.

    Refusing here rather than letting cc-rs fail three minutes in with a message
    that names a compiler nobody asked for."
    fi

    echo "smoke.sh: building the guest's binaries for $TARGET_TRIPLE (release)..."
    echo "smoke.sh: cross cc for ring: $CC_aarch64_unknown_linux_musl"
    # **Every one of them, unconditionally**, and that placement is the fix for
    # a defect this harness had for exactly one run: a block that built a binary
    # only when the file was ABSENT left a stale one in place, so a change was
    # made, the run repeated, and the guest served the previous build. The
    # failure was identical to the one being fixed, which is the worst way for a
    # loop to lie.
    if ! ( cd "$REPO_ROOT" && cargo build --release --target "$TARGET_TRIPLE" \
             -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-timed \
             -p rivetos-netd -p rivetos-mcpd ); then
        fail "cargo build --release --target $TARGET_TRIPLE for the guest's binaries failed - see build output above"
    fi

    # **And the CLIENT, for the host.** `cargo rivet run` is the thing an
    # operator actually uses (`AGENTS.md` §10), so the gate uses it rather than
    # a JSON-RPC probe written here — a harness that spoke its own dialect could
    # pass on a machine no real client could reach.
    CARGO_RIVET="$TARGET_DIR/release/cargo-rivet"
    echo "smoke.sh: building cargo-rivet for the HOST — the control-plane client..."
    if ! ( cd "$REPO_ROOT" && cargo build --release -p cargo-rivet ); then
        fail "cargo build --release -p cargo-rivet failed - see build output above. That is the control-plane client; without it this gate cannot ask the guest anything."
    fi
fi
if [ -n "$MCPD_BIN" ] && [ ! -f "$MCPD_BIN" ]; then
    fail "no binary at $MCPD_BIN — the fourth argument names something that is not there, or the build did not produce one."
fi
if [ -n "$MCPD_BIN" ] && [ -z "$SUP_BIN" ]; then
    fail "a control plane with no supervisor packs a binary onto a machine where nothing starts it — the service definition is the supervisor's (ADR-017 §4.1, ADR-035 phase 1), so there would be nothing to assert"
fi
if [ -n "$MCPD_BIN" ] && [ -z "$CLI_BIN" ]; then
    fail "a control plane with no CLI is a daemon that accepts a connection and then cannot spawn what it exists to hand over: cmd/rivet-mcpd gives each caller the stdin and stdout of \`rivet mcp\` (ADR-035 phase 1). Pack the CLI, or neither."
fi
if [ -n "$TIMED_BIN" ] && [ ! -f "$TIMED_BIN" ]; then
    fail "no binary at $TIMED_BIN — RIVET_TIMED names something that is not there, or the build did not produce one."
fi
if [ -n "$TIMED_BIN" ] && [ -z "$SUP_BIN" ]; then
    # Nothing would start it, so there would be nothing to assert.
    TIMED_BIN=
fi

[ -f "$INIT_BIN" ] \
    || fail "no binary at $INIT_BIN — the build did not produce one, or the given path is wrong"
if [ -n "$SUP_BIN" ] && [ ! -f "$SUP_BIN" ]; then
    fail "no binary at $SUP_BIN — the build did not produce one, or the given path is wrong"
fi
if [ -n "$CLI_BIN" ] && [ ! -f "$CLI_BIN" ]; then
    fail "no binary at $CLI_BIN — the build did not produce one, or the given path is wrong"
fi
if [ -n "$CLI_BIN" ] && [ -z "$SUP_BIN" ]; then
    fail "a CLI with no supervisor packs a login shell onto a machine where nothing renders /etc — the projection is the supervisor's (ADR-017 §3.1.8), so there would be no /etc/shells to name it in and nothing to assert"
fi
if [ -n "$NUSHELL_BIN" ] && [ ! -f "$NUSHELL_BIN" ]; then
    fail "no binary at $NUSHELL_BIN — RIVET_NUSHELL names something that is not there. tools/nushell/build.sh leaves it at \$HOME/.rivet/nushell/bin/nu."
fi
if [ -n "$NUSHELL_BIN" ] && [ -z "$SUP_BIN" ]; then
    fail "a nushell with no supervisor packs a login shell onto a machine where nothing renders /etc — the projection is the supervisor's (ADR-017 §3.1.8), so there would be no /etc/passwd for it to be root's shell in"
fi
# nu is FIRST in LOGIN_SHELL_PATHS, so packing it without the CLI would produce
# a card where /etc/shells names one shell rather than two, and the assertions
# below could not tell "the projection prefers nu" from "the projection found
# only nu". Refused rather than asserted around: the interesting claim is the
# ORDER, and an order needs two things.
if [ -n "$NUSHELL_BIN" ] && [ -z "$CLI_BIN" ]; then
    fail "a nushell with no CLI cannot demonstrate the dual-login-shell ruling's ordering: with only one candidate present, root getting nu says nothing about preference. Pack both, or neither."
fi

SUPERVISOR_PATH=$(constant "$SUPERVISE_RS" SUPERVISOR_PATH) || exit 1
# Where the projection probes. Read out of rivet-core because that is the
# constant cmd/rivet-sysd passes to file_exists — run.sh packs the CLI
# there for the same reason, and the console lines asserted below carry the
# path the *guest* looked at, so a disagreement between the two shows up as a
# failed assertion rather than as a silently weaker test.
CLI_PATH=$(constant "$CORE_ETC_RS" CLI_PATH) || exit 1
# And nushell's, which is the FIRST entry of LOGIN_SHELL_PATHS since
# the dual-login-shell ruling (2026-08-24). Read whether or not one is packed: the negative run
# asserts on the path the projection says it *would* have used, so the harness
# needs it in both directions.
NUSHELL_PATH=$(constant "$CORE_ETC_RS" NUSHELL_PATH) || exit 1

# Which shell this run expects root to end up with, derived rather than stated.
#
# The rule that orders the candidates: `/etc/passwd` has one seventh field and it takes the
# first entry of LOGIN_SHELL_PATHS that is present. Working that out here — from
# what this harness packed — is the only way the assertion can be wrong when the
# supervisor's probe is wrong, rather than agreeing with it by construction.
if [ -n "$NUSHELL_BIN" ]; then
    EXPECTED_SHELL=$NUSHELL_PATH
    EXPECTED_LISTED=2
else
    EXPECTED_SHELL=$CLI_PATH
    EXPECTED_LISTED=1
fi
# --- the control plane's own names and paths -------------------------------
#
# Read out of the crates that own them, for SUPERVISOR_PATH's reason exactly:
# the supervisor's definition is what starts `rivet-mcpd` and what tells it
# where its identity is, and `run.sh` places the binary and that directory from
# the same constants. A copy here would go stale silently and the failure would
# look like a guest that did not come up.
MCPD_NAME=$(constant "$DEFINITIONS_RS" MCPD_NAME) || exit 1
MCPD_PATH=$(constant "$DEFINITIONS_RS" MCPD_PATH) || exit 1
MCPD_IDENTITY_DIR=$(constant "$DEFINITIONS_RS" MCPD_IDENTITY_DIR) || exit 1
MCPD_PEERS_PATH=$(constant "$DEFINITIONS_RS" MCPD_PEERS_PATH) || exit 1
# **The port, from the schema and not from this file.** `PLAN.md` §92's fourth
# defect was three control-plane ports in three places, two of them wrong, and
# an operator copying an example had no way to tell which was real. There is one
# authority and it is `rivet_config::control_plane::DEFAULT_PORT`.
CONTROL_PORT=$(constant "$CONTROL_PLANE_RS" DEFAULT_PORT) || exit 1
case $CONTROL_PORT in
    ''|*[!0-9]*) fail "rivet_config::control_plane::DEFAULT_PORT read as '$CONTROL_PORT', which is not a port number — this harness will not guess one" ;;
esac
DEFAULT_LISTEN=$(constant "$CONTROL_PLANE_RS" DEFAULT_LISTEN) || exit 1

# --- the control-plane boot's prerequisites, checked before anything is built
#
# Checked here rather than at the stage itself, so that a run which cannot
# possibly finish says so in a second instead of after a QEMU boot.
NETWORK_DOC="$HERE/qemu-network.toml"
if [ -n "$MCPD_BIN" ]; then
    [ -f "$NETWORK_DOC" ] \
        || fail "the control-plane boot needs $NETWORK_DOC — the harness document matching QEMU's own user-mode network"

    # The client. Either the build above produced one, or a run that skipped the
    # build has to be told where it is: this is a HOST binary and there is no
    # aarch64 slot it could arrive in.
    if [ -z "$CARGO_RIVET" ]; then
        CARGO_RIVET=${RIVET_CARGO_RIVET:-$TARGET_DIR/release/cargo-rivet}
    fi
    [ -x "$CARGO_RIVET" ] \
        || fail "no cargo-rivet at $CARGO_RIVET. That is the control-plane client (AGENTS.md §10) and this gate has no other way to ask the guest anything. Build it with: cargo build --release -p cargo-rivet, or set RIVET_CARGO_RIVET."

    # And rivet-netd, without which the interface never takes the address the
    # forwarded port lands on. A machine without it boots perfectly, serves
    # nothing reachable, and the symptom is a connection refused — which looks
    # like a broken control plane and is not.
    if [ -z "$NETD_BIN" ]; then
        NETD_BIN=${RIVET_NETD:-$TARGET_DIR/$TARGET_TRIPLE/release/rivet-netd}
    fi
    [ -f "$NETD_BIN" ] \
        || fail "the control-plane boot needs rivet-netd at $NETD_BIN and there is nothing there. Without it the guest never gets an address and the forwarded port reaches nothing. Build it with: cargo build --release --target $TARGET_TRIPLE -p rivetos-netd, or set RIVET_NETD."
fi

# The pair has to stay up long enough for PID 1 to reach its confirmation
# decision, which it does not attempt until the supervisor has been up for
# CONFIRM_SETTLE_MS. Ten seconds is plenty for a lone PID 1 and would silently
# stop the interesting half of a paired run from ever happening.
if [ -n "$SUP_BIN" ]; then
    # Rust's numeric separators are not shell arithmetic; 30_000 is 30000.
    SETTLE_MS=$(constant "$SUPERVISE_RS" CONFIRM_SETTLE_MS | tr -d '_') || exit 1
    DEFAULT_TIMEOUT=$(( SETTLE_MS / 1000 + 10 ))
else
    DEFAULT_TIMEOUT=10
fi
TIMEOUT_SECS=${RIVET_SMOKE_TIMEOUT:-$DEFAULT_TIMEOUT}

if [ -n "$SUP_BIN" ]; then
    echo "smoke.sh: BOOT 1 — the machine with nothing configured."
    echo "smoke.sh: booting $INIT_BIN with $SUP_BIN at $SUPERVISOR_PATH (bounded to ${TIMEOUT_SECS}s)..."
    set -- "$INIT_BIN" --supervisor "$SUP_BIN"
    if [ -n "$CLI_BIN" ]; then
        echo "smoke.sh: and $CLI_BIN at $CLI_PATH, where the /etc projection probes."
        set -- "$@" --cli "$CLI_BIN"
    else
        echo "smoke.sh: with NO CLI packed, so the projection must report that /etc/shells names no shell."
    fi
    if [ -n "$NUSHELL_BIN" ]; then
        echo "smoke.sh: and $NUSHELL_BIN at $NUSHELL_PATH, first in LOGIN_SHELL_PATHS —"
        echo "smoke.sh: so root's shell must be nu and /etc/shells must name two."
        set -- "$@" --nushell "$NUSHELL_BIN"
    elif [ -n "$CLI_BIN" ]; then
        echo "smoke.sh: with NO nushell packed, so the probe must fall through to the CLI."
        echo "smoke.sh: set RIVET_NUSHELL to tools/nushell/build.sh's output for the other direction."
    fi
    if [ -n "$TIMED_BIN" ]; then
        echo "smoke.sh: and $TIMED_BIN, which emits ADR-022 §3 RECORDS rather than"
        echo "smoke.sh: lines — the only producer of §4's first capture path."
        set -- "$@" --timed "$TIMED_BIN"
    fi
    # **No control plane on this boot, and that is the assertion.** Boot 1 is a
    # machine with nothing configured, so the supervisor's `rivet-mcpd`
    # definition — which is `enabled: true` — must find no binary and REFUSE by
    # name. Packing it here would make the negative direction unreachable and
    # would start a daemon with no identity, which exits 2 in a loop.
    echo "smoke.sh: with NO $MCPD_NAME packed, so its spawn must REFUSE with ENOENT"
    echo "smoke.sh: and this machine must have no route in at all. BOOT 2 is the other way."
else
    echo "smoke.sh: booting $INIT_BIN alone, with NO supervisor packed (bounded to ${TIMEOUT_SECS}s)."
    echo "smoke.sh: this exercises PID 1's give-up path only; pass a second binary for the whole system."
    set -- "$INIT_BIN"
fi
echo

# run.sh itself execs qemu-system-aarch64 (no intervening fork), so `timeout`
# signals QEMU directly. -k gives it 2s to honour SIGTERM before SIGKILL.
# GNU timeout exits 124 exactly when it had to kill the child for running
# past the deadline — that is how we tell "still parked" from "exited on its
# own", the fourth assertion below.
#
# </dev/null is not decoration: -nographic puts the guest's console on stdio,
# so QEMU *reads* this script's stdin. Run from a shell that is itself being
# fed a script on stdin and QEMU eats the rest of it, and the run ends early
# for a reason nothing on the console explains.
set +e
timeout -k 2 "$TIMEOUT_SECS" "$HERE/run.sh" "$@" >"$LOG" 2>&1 </dev/null
RC=$?
set -e

# --- Kernel panic - not syncing: Attempted to kill init! is THE failure mode
# of this project (ADR-026 §1). Check it first: -no-reboot means QEMU does
# not exit the host process on a guest panic (nothing here calls poweroff),
# so this text is the only signal — RC alone would still read as "timed out
# while parked" and miss it entirely.
if grep -q "Kernel panic" "$LOG"; then
    PANIC_LINE=$(grep "Kernel panic" "$LOG" | head -1)
    EXITCODE=$(printf '%s\n' "$PANIC_LINE" | grep -oE 'exitcode=0x[0-9a-fA-F]+' | head -1 | tr 'A-Z' 'a-z')
    case "$EXITCODE" in
        exitcode=0x0000000b)
            fail "PID 1 panicked — $PANIC_LINE
    $EXITCODE is SIGSEGV: rivet-initd faulted (segfault or similar). This needs debugging, not a config change."
            ;;
        exitcode=0x00000000)
            fail "PID 1 panicked — $PANIC_LINE
    $EXITCODE means main() simply returned. PID 1 must never return (ADR-017 §3.1.5) — find the path that falls out of main instead of parking."
            ;;
        "")
            fail "PID 1 panicked — $PANIC_LINE
    (no 'exitcode=0x...' found on that line to decode)"
            ;;
        *)
            fail "PID 1 panicked — $PANIC_LINE
    $EXITCODE — not one of the two documented codes (0x0000000b SIGSEGV, 0x00000000 returned); decode it against the kernel's arm64 fault/signal numbering."
            ;;
    esac
fi

if [ "$RC" -ne 124 ]; then
    fail "QEMU (via run.sh) exited with status $RC before the ${TIMEOUT_SECS}s timeout, instead of being stopped by it. Expected it to still be running (PID 1 parked) at the deadline. Either run.sh refused to start (see output above) or QEMU exited on its own."
fi

# **The path is derived, not written here.** It used to be the literal `/init`,
# and that assertion outlived the reason for it: `tools/qemu/run.sh` now packs
# pid 1 where a board puts it, so the kernel announces THAT path. A hard-coded
# string would have this test fail on a correct boot - which is exactly what it
# did, once, and the failure said the kernel never reached init at all.
INIT_HANDOFF="Run $(constant "$REPO_ROOT/cmd/rivet-image/src/system.rs" INIT_PATH) as init process"
if ! grep -qF "$INIT_HANDOFF" "$LOG"; then
    fail "console never showed the kernel line '$INIT_HANDOFF' — the kernel did not reach the init handoff at all"
fi

if ! grep -qF "rivet-initd: starting as pid 1" "$LOG"; then
    fail "console never showed 'rivet-initd: starting as pid 1' — init did not run, printed something else, or this is not the expected binary"
fi

if [ -z "$SUP_BIN" ]; then
    echo
    echo "smoke.sh: PASS (PID 1 alone — no supervisor was packed)"
    echo "  - kernel reached '$INIT_HANDOFF'"
    echo "  - console showed 'rivet-initd: starting as pid 1'"
    echo "  - no 'Kernel panic' in the console output"
    echo "  - QEMU was still running (PID 1 parked) when the ${TIMEOUT_SECS}s timeout stopped it"
    echo
    echo "  NOT asserted, because nothing was packed at $SUPERVISOR_PATH:"
    echo "  the supervisor starting and staying up, PID 1 reaching its boot"
    echo "  confirmation rather than withholding it, and /etc being rendered at"
    echo "  all — the projection is the supervisor's (ADR-017 §3.1.8), so this"
    echo "  boot has the empty /etc every Rivet had before 2026-08-23. Nothing"
    echo "  about the login path either, in either direction, for the same"
    echo "  reason: the file that would name a login shell is not written on this"
    echo "  boot at all. Pass a second binary for those."
    exit 0
fi

# --- the paired boot: the half that could never be reached before ---

if grep -qF "rivet-initd: cannot spawn $SUPERVISOR_PATH" "$LOG"; then
    fail "PID 1 could not spawn $SUPERVISOR_PATH even though a binary was packed there. The initramfs path and the path compiled into cmd/rivet-initd have diverged, or the binary is not executable."
fi

if ! grep -qF "rivet-initd: started $SUPERVISOR_PATH as pid" "$LOG"; then
    fail "console never showed 'rivet-initd: started $SUPERVISOR_PATH as pid N' — PID 1 never reported spawning its child"
fi

if ! grep -qF "supervisor starting" "$LOG"; then
    fail "the supervisor never announced itself on the console. PID 1 reported spawning it, so it started and then said nothing — check that its journal reaches stdout, and that stdout is the console PID 1 inherited"
fi

# This line is only ever printed when the supervisor has exited. In a healthy
# run it must not appear at all: the point of the paired boot is a child that
# starts and STAYS up.
if grep -qF "rivet-initd: supervisor pid " "$LOG"; then
    fail "the supervisor exited during the run — $(grep -F 'rivet-initd: supervisor pid ' "$LOG" | head -1)"
fi

if grep -qF "GIVING UP" "$LOG"; then
    fail "PID 1 exhausted its retry budget and gave up on the supervisor, in a run where a supervisor was packed"
fi

# --- the cgroup2 authority: ADR-017 §3.1.3.1, added 2026-08-23 ---
#
# Two assertions, opposite directions, and the negative one is the whole point.
# A board printed "cannot read the service cgroup … would double-start it after
# a supervisor restart" once per service on a boot where cgroup2 was mounted and
# nothing was wrong (§3.1.3.1). The hierarchy was there; the SERVICE's directory
# was not, because the service had not run yet.
#
# So a healthy paired boot must now say the machine-wide fact ONCE, at info, and
# say NOTHING about either service's cgroup. An assertion that only checked for
# the positive line would have passed on every day the defect existed.

if ! grep -qF '"msg":"the cgroup2 hierarchy is readable' "$LOG"; then
    fail "the supervisor never reported reading the cgroup2 hierarchy. ADR-017 §3.1.3 makes a service's cgroup THE authority for what is running, and §3.1.3.1 has the supervisor state once, at start-up, whether that authority exists. Either PID 1 did not mount cgroup2 (check the mount table lines above), or the start-up probe was removed from cmd/rivet-sysd's run()."
fi

if grep -qF 'DEGRADED there is no readable cgroup2 hierarchy' "$LOG"; then
    fail "the supervisor found no cgroup2 hierarchy, in a run where PID 1's stage0_plan mounts one — $(grep -F 'DEGRADED there is no readable cgroup2 hierarchy' "$LOG" | head -1)
    With no hierarchy there is no authority for what is running, so a restarted supervisor starts a second copy of every service that survived it (ADR-017 §3.1.3)."
fi

if grep -qF 'cannot be established' "$LOG"; then
    fail "a service reported that whether it is already running cannot be established, on a machine whose cgroup2 hierarchy is readable — $(grep -F 'cannot be established' "$LOG" | head -1)
    ADR-017 §3.1.3.1: a first boot has an absent service cgroup for the ordinary reason that the service has not run yet, and that is not news. This line means either the hierarchy really is unreadable, or the benign case has started warning again."
fi

# --- /etc, which nothing rendered before 2026-08-23 ---
#
# ADR-007 §3.5 gives /etc exactly one generator, the runtime projection, and
# ADR-017 §3.1.8 puts it in the supervisor, before it converges any service.
# Until it existed a running Rivet had NO /etc at all, so every login was
# rejected before authentication.
#
# Every assertion below reads a value the SUPERVISOR read back off the kernel,
# not one it intended. That distinction is the whole reason these lines carry
# `read_back_bytes` and a mount-table line at all.

if grep -qF 'REFUSED /etc could not be mounted' "$LOG"; then
    fail "the /etc projection could not mount its tmpfs — $(grep -F 'REFUSED /etc could not be mounted' "$LOG" | head -1)
    With no /etc, musl reads none of its nine compiled-in paths (ADR-008 §5.1): getpwnam(\"root\") answers nothing, localhost resolves over DNS, and getusershell() claims /bin/sh and /bin/csh are this machine's login shells."
fi

if ! grep -qF '"msg":"/etc projected"' "$LOG"; then
    fail "the supervisor never reported projecting /etc. It announced itself, so it started and then did not reach crate::etc::project — check that the call still precedes the first converge() in cmd/rivet-sysd/src/main.rs"
fi

# How many files, read out of rivet-core rather than written here, for the same
# reason SUPERVISOR_PATH is: a copy goes stale silently and the failure looks
# like a broken boot rather than a stale test.
#
# ~~Five~~ six since 2026-08-26 (`/etc/motd`, ADR-035 §9B.1) — and the number is
# not written here, which is why that ruling cost this line nothing but a
# comment. The test it scrapes lost its own number the same day.
EXPECTED_ETC=$(sed -n '/fn the_projection_renders_these_etc_files_and_no_others/,/^    }/p' \
    "$CORE_ETC_RS" | grep -c '"/etc/')
if [ "$EXPECTED_ETC" -lt 1 ]; then
    fail "cannot read the expected /etc file list out of $CORE_ETC_RS — refusing to guess how many files should have been rendered"
fi
RENDERED=$(grep -cF '"msg":"rendered"' "$LOG")
if [ "$RENDERED" -ne "$EXPECTED_ETC" ]; then
    fail "the supervisor rendered $RENDERED of $EXPECTED_ETC files under /etc. rivet-core defines $EXPECTED_ETC; a partial /etc is the state most likely to be misread as working, because getpwnam answers and getusershell does not."
fi

# `write(2)` returning zero is the supervisor's claim; the file is the kernel's.
if grep -qF 'read back is NOT what was written' "$LOG"; then
    fail "a file under /etc read back differently from what was written — $(grep -F 'read back is NOT what was written' "$LOG" | head -1)"
fi

# ADR-007 §3.6.1 item 1, which that section calls non-negotiable: the mount is
# genuinely read-only, so a hand edit FAILS rather than being accepted and
# discarded at the next boot. `kernel_says_read_only` is parsed out of
# /proc/self/mounts by the supervisor, so this is the kernel's word.
if grep -qF 'DEGRADED /etc is mounted WRITABLE' "$LOG"; then
    fail "/etc was left writable. ADR-007 §3.6's table calls a write that succeeds and is discarded at reboot the worst outcome available, and that is the state this boot is in."
fi
if ! grep -qF '"kernel_says_read_only":true' "$LOG"; then
    fail "the kernel's mount table never showed /etc as read-only. Either the remount was not attempted, or it was attempted, reported success, and did not take — which only /proc/self/mounts can tell apart."
fi

# --- the journal reaches the card — ADR-022 §7, §8, added 2026-08-24 ---
#
# **QEMU can settle this one, and that is not obvious.** ADR-026 §4 puts /state
# in the column only a board can answer, and it is right about the ext4
# partition — there is no /dev/mmcblk0 here, so PID 1's mount refuses. What is
# left at /state is a directory on the INITRAMFS, which is writable, so the
# journal opens on it and every record goes to a file.
#
# What that settles: the directory is created, the buffered records are flushed,
# and the write path runs against a real filesystem for a whole boot. What it
# does NOT settle is the half the name suggests — **persistence**. This /state
# is RAM and vanishes with the guest, so `--boot previous` has never selected a
# boot that actually ended. Only a board settles that.
if ! grep -qF '"msg":"the journal is on the card' "$LOG"; then
    fail "the supervisor never reported opening the journal on /state. ADR-022 §7 buffers records until /state is available and flushes them then; without that record, either the open was not attempted or it refused without saying so — and this boot's records are on the console and nowhere else."
fi
if grep -qF '"msg":"the journal has NO file on the card' "$LOG"; then
    fail "the journal refused /state — $(grep -F '"msg":"the journal has NO file on the card' "$LOG" | head -1)
    Under this emulator /state is a directory on the initramfs and IS writable, so a refusal here is a real defect rather than ADR-026 §4's missing block device."
fi
# The flush is the half that is easy to get wrong and impossible to see: the
# records from before /state was reachable are the ones that explain how the
# boot got there, and a journal that opened without draining its ring would look
# identical on the console.
FLUSHED=$(grep -F '"msg":"the journal is on the card' "$LOG" | head -1 \
    | sed -n 's/.*"flushed":\([0-9]*\).*/\1/p')
if [ -z "$FLUSHED" ] || [ "$FLUSHED" -lt 1 ]; then
    fail "the journal opened on /state and flushed ${FLUSHED:-no} buffered records. Everything before the open is emitted before it — the clock restore says at least one thing on every boot — so a flush of zero means ADR-022 §7's buffer is not being drained."
fi
if grep -qF '"msg":"the journal could NOT write to the card' "$LOG"; then
    fail "the journal opened and then failed to write — $(grep -F '"msg":"the journal could NOT write to the card' "$LOG" | head -1)"
fi

# --- ADR-022 §4's capture — added 2026-08-24 ---
#
# The half that is only reachable with a service that actually says something.
# `rivet-netd` says one thing on every boot without a document ("cannot read
# the configuration document"), which makes it the reliable producer here; the
# assertion is on the *shape* rather than on that sentence, because it is netd's
# to change.
#
# Both directions are asserted where each is reachable — with a service packed,
# the wrap must happen; the negative is the whole run before 2026-08-24 and is
# not reproducible without reverting the change, so what stands in for it is
# `cmd/rivet-sysd`'s own tests over `Capture`.
# **`$TIMED_BIN` and not `$NETD_BIN`** — corrected 2026-08-30. This used to ask
# about services BOOT 1 no longer packs: netd belongs to the control-plane boot
# and the other is gone entirely. `rivet-timed` is the service this
# boot actually runs, so it is the one whose output there is to capture, and
# asking about a binary that is merely BUILT would have made this whole block
# silently unreachable while looking exactly like a passing test.
if [ -n "$TIMED_BIN" ]; then
    if ! grep -q '"stream":"std' "$LOG"; then
        fail "a service ran and NOT ONE line of its output was wrapped as a record. ADR-022 §4's second capture path is what puts a mechanism's own words on /state/log rather than only on this console; without it, 'rivet get log' shows what the supervisor said about a service and nothing the service said."
    fi
    # And the record is in the SERVICE's name, not the supervisor's — which is
    # what makes `where svc == rivet-timed` select a mechanism's own words.
    if grep '"stream":"std' "$LOG" | grep -q '"svc":"rivet-sysd"'; then
        fail "a captured line was attributed to the supervisor rather than to the service that wrote it: $(grep '"stream":"std' "$LOG" | grep '"svc":"rivet-sysd"' | head -1)"
    fi
fi
# --- ADR-022 §4.1's kernel messages — the tripwire FIRED, and this is the other side of it ---
#
# **Until 2026-08-24 this asserted the opposite**, and the comment it replaces is
# worth keeping in view because it was right: `/dev/kmsg` did not exist under
# this emulator, `run.sh` boots an initramfs, and the kernel's
# `CONFIG_DEVTMPFS_MOUNT` does not apply to one — so `/dev` was an empty
# directory and the highest-value diagnostic source in the project could only
# ever be proved against the build host's kernel.
#
# It said: *"a fix makes the second branch fire and say so, rather than landing
# unnoticed"*, and asked whoever fixed it to invert the case and add assertions
# for what the kernel's records must contain. That is what this is. PID 1 now
# mounts `devtmpfs` itself (`rivet_core::mount`, the maintainer's 2026-08-24 ruling), so
# `/dev/kmsg` is present here and the ingest is live under the emulator.
#
# The direction is now: kernel records MUST be present, and the supervisor
# saying it cannot read kmsg is the failure.
if grep -q '"svc":"kernel"' "$LOG"; then
    # ADR-022 §4.1 forwards each `/dev/kmsg` line as a record. Assert the shape
    # rather than merely the presence, because "svc":"kernel" with an empty or
    # unparsed message would satisfy a bare grep and diagnose nothing.
    if ! grep '"svc":"kernel"' "$LOG" | grep -q '"msg":"[^"]'; then
        fail "kernel records are in the journal with empty messages — kmsg.rs is producing records out of lines it did not parse: $(grep '"svc":"kernel"' "$LOG" | head -1)"
    fi
    # §4.1 maps the eight syslog levels onto four severities. A boot that
    # produces only `info` means `severity_of` collapsed everything, which
    # would bury exactly the driver and firmware failures this exists to catch.
    if ! grep '"svc":"kernel"' "$LOG" | grep -qE '"sev":"(info|warn|error|debug)"'; then
        fail "kernel records carry no recognised severity — ADR-022 §4.1's mapping of the eight syslog levels onto four is not being applied: $(grep '"svc":"kernel"' "$LOG" | head -1)"
    fi
    # The kernel always says something identifiable early. If the only records
    # are late ones, the reader opened kmsg without seeking to the start and
    # every message from before the supervisor existed is lost — which on a
    # board is the whole boot.
    if ! grep '"svc":"kernel"' "$LOG" | grep -qiE '"msg":"[^"]*(Linux version|Booting|Machine model|command line)'; then
        fail "kernel records exist but none of them is from early boot. ADR-022 §4.1's reader is not picking up the ring buffer's existing contents, so on a board every driver probe and firmware load before the supervisor started would be lost: $(grep '"svc":"kernel"' "$LOG" | head -2)"
    fi
elif grep -qF '"msg":"the kernel'"'"'s own messages are NOT in this journal' "$LOG"; then
    fail "the supervisor says it cannot read /dev/kmsg — which was true here until 2026-08-24 and is now a REGRESSION.

    PID 1 mounts devtmpfs (rivet_core::mount::stage0_plan), so /dev/kmsg exists under
    this emulator. If the supervisor cannot open it, either the mount is failing (check
    for a Degraded boot verdict in the log above) or the reader is refusing for some
    other reason. On a board this means every driver probe, every firmware that did not
    load and every filesystem that refused reaches the console and nowhere else."
else
    fail "the supervisor said nothing about /dev/kmsg at all. ADR-022 §4.1's ingest is either not attempted or refusing silently — and on a board that would mean every driver probe, every firmware that did not load and every filesystem that refused reaches the console and nowhere else."
fi

# --- /dev itself, asserted directly rather than only through its consequences ---
#
# The three failures that were only ever caused by an empty `/dev` cost hours
# each and none of them named `/dev` in its error. `Stdio::null()` opens
# `/dev/null` in the PARENT before the fork, so every spawn failed ENOENT
# against a binary that was present. This asserts the cause, so the next
# occurrence is diagnosed in one line rather than in four hours.
if ! grep -qE '"target":"/dev"|"msg":"[^"]*devtmpfs' "$LOG"; then
    printf 'note: no devtmpfs mount record found in the log; /dev assertions rest on the journal alone\n' >&2
fi
if [ -n "$TIMED_BIN" ]; then
    # ADR-022 §4's FIRST path. `rivet-timed` emits records rather than lines,
    # so the capture must parse one back and forward it — keeping its severity.
    # Wrapping it as text would bury a failed time client at `info`.
    if ! grep -qF '"wrapped":"record"' "$LOG"; then
        fail "rivet-timed ran and none of its records was forwarded as a record. ADR-022 §4's first path is what keeps a service's own severity: wrapped as text, an error from the time client reads as info and 'rivet get log | where sev == error' misses it."
    fi
    if ! grep -F '"wrapped":"record"' "$LOG" | grep -q '"svc":"rivet-timed"'; then
        fail "a forwarded record was not attributed to rivet-timed — the supervisor overrides svc with the service it started, so this means the wrong service was credited: $(grep -F '"wrapped":"record"' "$LOG" | head -1)"
    fi
fi

# --- the login path, in whichever direction this run selected ---
#
# The projection reads one fact off the machine rather than out of a constant:
# which of LOGIN_SHELL_PATHS are there. Both answers are asserted, each in
# the run that produces it, and the console lines below carry the path the
# GUEST probed — so a harness packing to one path while the supervisor looks at
# another fails here instead of quietly asserting less.
#
# A probe that could not be answered is a third outcome and is a failure in
# both directions. Without this, an EIO from file_exists would satisfy the
# negative run's assertions for entirely the wrong reason.
if grep -qF '"msg":"could not tell whether a login shell is present' "$LOG"; then
    fail "the login-shell probe could not be answered — $(grep -F 'could not tell whether a login shell is present' "$LOG" | head -1)
    That is neither of the two outcomes this run distinguishes: /etc/shells names nothing, and the reason is a failed lookup rather than an absent binary."
fi

if [ -n "$CLI_BIN" ]; then
    if ! grep -qF '"msg":"/etc/shells names a login shell' "$LOG"; then
        fail "a CLI was packed at $CLI_PATH and the projection did not report finding a login shell. The probe is a file_exists on that exact path (cmd/rivet-sysd's etc.rs), so either run.sh packed it somewhere else or the binary is not there."
    fi
    # The SHELL field is root's — /etc/passwd's seventh, the first present entry
    # of LOGIN_SHELL_PATHS. EXPECTED_SHELL was derived above from what this
    # harness packed, so a supervisor that took the *last* candidate rather than
    # the first fails here rather than agreeing with itself.
    if ! grep -qF "\"shell\":\"$EXPECTED_SHELL\"" "$LOG"; then
        fail "the projection named a login shell, but not $EXPECTED_SHELL — $(grep -F '"msg":"/etc/shells names a login shell' "$LOG" | head -1)
    With nu packed, the rule that orders the candidates makes it root's shell; without it, the CLI is."
    fi
    # And how many /etc/shells names, which is the half the SHELL field cannot
    # carry: a projection that took nu and *dropped* the CLI would satisfy the
    # assertion above and would quietly remove the shell that same rule keeps
    # listed for a future restricted account.
    if ! grep -qF "\"listed\":$EXPECTED_LISTED" "$LOG"; then
        fail "/etc/shells should name $EXPECTED_LISTED shell(s) in this run and the projection reported otherwise — $(grep -F '"msg":"/etc/shells names a login shell' "$LOG" | head -1)"
    fi
    if [ -n "$NUSHELL_BIN" ] && ! grep -qF "\"also_permitted\":\"$CLI_PATH\"" "$LOG"; then
        fail "nu is root's shell and $CLI_PATH is not also permitted. The rule that keeps every present shell listed keeps the CLI in /etc/shells so that an account naming it is not refused before authentication — dropping it is a change nobody ruled."
    fi
    if grep -qF '"msg":"/etc/shells names NO login shell' "$LOG"; then
        fail "the projection reported BOTH that it named a shell and that it named none, in one boot"
    fi
    if ! grep -qF '"permits_a_login":true' "$LOG"; then
        fail "/etc/shells named $EXPECTED_SHELL, and the supervisor still reported \"permits_a_login\":false. Those two come from the same list, so they cannot disagree — read the '/etc projected' line in the console below."
    fi
else
    # The honest negative, and it is a state a real card can be in: an image
    # built without a CLI, or one where the install path moved. Losing the
    # ability to reproduce it would be losing the only run in which the
    # projection's refusal to name an absent binary is visible.
    if ! grep -qF '"msg":"/etc/shells names NO login shell' "$LOG"; then
        fail "no CLI was packed, and the projection did not report that /etc/shells names none. Either something else is at $CLI_PATH in this initramfs, or the projection stopped saying so — and an /etc/shells naming a binary that is not there fails at exec AFTER authentication."
    fi
    # `would_be` carries every candidate, space-separated, since the dual-login-shell ruling
    # made LOGIN_SHELL_PATHS a list. Asserted as containment rather than as the
    # exact joined string: the ORDER is asserted in rivet-core's own suite,
    # and pinning the separator here would make this harness fail on a change to
    # the record's formatting rather than to its meaning.
    for candidate in "$NUSHELL_PATH" "$CLI_PATH"; do
        if ! grep -F '"would_be":' "$LOG" | grep -qF "$candidate"; then
            fail "the projection named no login shell but did not report $candidate among the paths it looked at. This harness and the running supervisor disagree about where a login shell would be."
        fi
    done
    if grep -qF '"msg":"/etc/shells names a login shell' "$LOG"; then
        fail "the projection found a login shell in a boot where none was packed — $(grep -F '"msg":"/etc/shells names a login shell' "$LOG" | head -1)"
    fi
    if ! grep -qF '"permits_a_login":false' "$LOG"; then
        fail "no CLI was packed and the supervisor did not report \"permits_a_login\":false"
    fi
fi

# --- the control plane's NEGATIVE direction, added 2026-08-30 ---------------
#
# **Boot 1 has no route in, and this is the assertion that says so out loud.**
#
# `cmd/rivet-sysd`'s `mcpd` definition is `enabled: true` since 2026-08-29, so a
# machine that does not carry the binary must try to start it and refuse — once,
# permanently, naming the service. `ADR-018` §3 makes `/system` read-only, so no
# later attempt could find what this one did not, and `supervise.rs` marks it
# failed without retrying for exactly that reason.
#
# The positive direction is BOOT 2. Both are asserted, each in the boot that
# produces it (`AGENTS.md` §4), and the negative is the state every card built
# before `cmd/rivet-image` carried this binary was really in.
#
# **No service on this image has a precondition, and this is the tripwire.**
# The one that did was removed with the mechanism it belonged to (`ADR-035`
# §13), and the assertions here used to require a `"REFUSED a precondition"`
# record on every boot — so this gate had been FAILING since that ruling, and
# nothing noticed because every check since went straight to hardware. That is
# `PLAN.md` §102.5's finding twice over, and the direction is now inverted: a
# precondition RUNNING is what this fails on.
if grep -qF '"msg":"precondition met"' "$LOG"; then
    fail "a precondition was met on this boot, and no service on this image has one. Either a service grew one and this assertion is out of date, or something is running that should not be: $(grep -F '"msg":"precondition met"' "$LOG" | head -1)"
fi

if ! grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | grep -qF 'REFUSED this image cannot run'; then
    echo >&2
    echo "--- every line the supervisor wrote about $MCPD_NAME ---" >&2
    grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" >&2 || echo "      (not one)" >&2
    fail "no $MCPD_NAME was packed and the supervisor did not refuse to start one.

    Its definition is enabled: true (ADR-035 phase 1, flipped 2026-08-29), so on a
    machine whose image lacks $MCPD_PATH the spawn must fail ENOENT and be reported
    once, permanently, naming the service. Silence here means either the definition
    went back to enabled: false — which is what left a card booting with NO CONTROL
    PLANE for three whole phases (PLAN.md §94.7) — or something else is at that
    path in this initramfs."
fi
# **And what the machine LOST**, not merely which call failed. `ADR-017`
# §3.1.6's `consequence_of` is what turns a refusal into something an operator
# can act on, and for this service the consequence is the whole administration
# surface — so a refusal that did not carry one would be the most expensive
# possible silence.
if ! grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | grep -qF '"consequence"'; then
    fail "the control plane's spawn refused and the record carries no consequence. ADR-017 §3.1.6 has the supervisor say what the machine has lost, and for this service that is: there is no way in at all: $(grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | head -1)"
fi
if grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | grep -qF '"msg":"started"'; then
    fail "the control plane STARTED in a boot where no binary was packed — something else is at $MCPD_PATH in this initramfs: $(grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | grep -F '"msg":"started"' | head -1)"
fi

# ADR-017 §3.1.4/§3.1.6: 'boot NOT confirmed' is the *gate* refusing — the
# mount phase, the provisioning document or the supervisor was unacceptable, so
# PID 1 never even looked at the tryboot flag. That is the outcome the
# supervisor-less boot always produced, and the one this run must not.
if grep -qF "boot NOT confirmed:" "$LOG"; then
    fail "PID 1 withheld the boot confirmation — $(grep -F 'boot NOT confirmed:' "$LOG" | head -1)"
fi

# What it must do instead is *reach a verdict*. Under QEMU that verdict is
# necessarily a failed read of the tryboot flag: ADR-026 §4 puts tryboot and
# /proc/device-tree/chosen/bootloader squarely in the board's column — "not
# emulated at all" — so `Confirmed` is not reachable here and asserting it
# would be asserting a lie. What IS settled here is everything up to it: the
# gate passed, the supervisor stayed up for CONFIRM_SETTLE_MS, and PID 1 went
# looking for the flag.
if ! grep -qE "rivet-initd: (boot confirmed:|boot could NOT be confirmed:|not a tryboot boot|partition [0-9]+ is already the default)" "$LOG"; then
    fail "PID 1 never reached a boot-confirmation verdict at all within ${TIMEOUT_SECS}s. It should have attempted one once the supervisor had been up for CONFIRM_SETTLE_MS."
fi

# --- BOOT 2: the machine ANSWERING, over ADR-035's control plane ------------
#
# Everything above proves a Rivet BOOTS. This proves something can ASK IT A
# QUESTION, which is the whole of what `ADR-035` left the harness unable to
# do: the channel `smoke.sh`'s only route into a guest went through came out of
# the image on 2026-08-29, and for a day the gate that runs before every
# deploy could boot a machine and not speak to it (`PLAN.md` §102.5).
#
# It is a second QEMU because it needs four things BOOT 1 deliberately does not
# have: a network, a forwarded port, a stored document, and an identity.
#
# **The client is `cargo rivet run`** — the same binary an operator uses
# (`AGENTS.md` §10), not a JSON-RPC probe written here. A harness that spoke its
# own dialect could pass against a machine no real client could reach, which is
# the shape of green this project spends the most effort removing.
CP_STATUS_OUT=
CP_CATALOGUE_OUT=
CP_HOSTS_OUT=
CP_WRONG_PIN_ERR=
STOPPED=
HOSTNAME_PROBE=smoke-guest
if [ -n "$MCPD_BIN" ]; then
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-smoke-cp.XXXXXX") \
        || fail "could not create a temporary directory for the control-plane boot"
    chmod 700 "$WORK"
    trap 'rm -f "$LOG"; rm -rf "$WORK"' EXIT

    echo
    echo "smoke.sh: BOOT 2 — the machine answering over the control plane."

    # --- two THROWAWAY identities, minted per run ---------------------------
    #
    # See this file's header for the argument. In short: `AGENTS.md` §8.3 keeps
    # key material out of tracked files, and a committed `device.seed` would be
    # a private key every reader of this repository holds.
    #
    # `cargo rivet identity new` is the only thing in the workspace that can
    # mint one — `ADR-025` §3 puts generation in the fleet tool and `rivetos-tls`
    # cannot generate at all — so this is also the only way for the harness to
    # have one.
    echo "smoke.sh: minting THROWAWAY identities in $WORK (guest and operator)"
    "$CARGO_RIVET" identity new --directory "$WORK/guest" --name rivet-smoke-guest \
        >"$WORK/mint-guest" 2>&1 \
        || { cat "$WORK/mint-guest" >&2; fail "cargo rivet identity new could not mint the GUEST's identity"; }
    "$CARGO_RIVET" identity new --directory "$WORK/operator" --name rivet-smoke-operator \
        >"$WORK/mint-operator" 2>&1 \
        || { cat "$WORK/mint-operator" >&2; fail "cargo rivet identity new could not mint the OPERATOR's identity"; }

    # The pins, read back out of the certificates rather than scraped from the
    # minting output: `identity show` exists for exactly this and reads the
    # certificate, never the seed.
    GUEST_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/guest" 2>&1 \
        | grep -oE '[0-9a-f]{64}' | head -1)
    OPERATOR_PIN=$("$CARGO_RIVET" identity show --directory "$WORK/operator" 2>&1 \
        | grep -oE '[0-9a-f]{64}' | head -1)
    [ -n "$GUEST_PIN" ] \
        || fail "could not read the guest identity's public key back out of its certificate. \`cargo rivet identity show\` is what prints it, and without a pin there is nothing for the client to trust."
    [ -n "$OPERATOR_PIN" ] \
        || fail "could not read the operator identity's public key back out of its certificate"
    # Two identities that came out the same would make the whole exchange below
    # pass while proving nothing about who is who — and it is the kind of thing
    # a broken `seed_of` produces silently.
    [ "$GUEST_PIN" != "$OPERATOR_PIN" ] \
        || fail "the guest and the operator minted the SAME public key, so nothing below would distinguish one end from the other"

    # --- what goes on the guest's /state ------------------------------------
    #
    # `ADR-025` §2's trust is mutual and symmetric, and this is both halves of
    # it: the guest holds its own seed and certificate, and a pin file naming
    # the one key that may connect.
    mkdir -p "$WORK/secrets"
    cp "$WORK/guest/device.seed" "$WORK/guest/device.crt" "$WORK/secrets/" \
        || fail "could not assemble the guest's identity directory"
    printf '# one hex public key per line — %s\n%s\n' \
        "the operator this run minted" "$OPERATOR_PIN" \
        >"$WORK/secrets/${MCPD_PEERS_PATH##*/}" \
        || fail "could not write the guest's pin file"

    # --- the document -------------------------------------------------------
    #
    # QEMU's own network, a name, and an address for the control plane to bind.
    #
    # **`0.0.0.0` and not the built-in default**, and that is the point of the
    # table: `rivet_config::control_plane::DEFAULT_LISTEN` is
    # $DEFAULT_LISTEN, and SLIRP has no route to a loopback listener
    # inside the guest — a forwarded port would connect to nothing and the
    # failure would read as a control plane that never started.
    #
    # **`hostname` is inserted after `version`, not appended.** A bare key
    # written after a `[table]` header belongs to that table, so appending it
    # would produce `network.eth0.hostname` and the document would not mean what
    # it says. `[control-plane]` is itself a table header, so it goes at the end
    # safely — the two are not the same case and are written differently on
    # purpose.
    #
    # **The fixture's own `version` is removed by PATTERN, and its carriage
    # returns with it.** This was `grep -v '^version = 1$'`, and that filter is
    # exactly one character away from working: the fixture is a tracked file on
    # a Windows checkout, `$` does not match before a `\r`, so the line survived
    # and the composed document carried `version` TWICE. `rivet-mcpd` refused it
    # with *"duplicate key"* at a line number inside the fixture, restarted five
    # times and gave up — a control plane that never listened, for a reason
    # nothing about the control plane. `.gitattributes` now pins the fixture to
    # LF as well; this is the half that does not depend on a checkout.
    DOC="$WORK/machine.toml"
    CONTROL_LISTEN="0.0.0.0:$CONTROL_PORT"
    # The window this boot declares. Two times that cannot be confused
    # with each other or with a default, so a file holding anything
    # else is visibly wrong rather than plausibly right.
    # `rivet_core::display::HOURS_PATH`, restated here because this harness
    # cannot link the crate. A drift makes this stage fail loudly, which is
    # the correct outcome for a path that moved.
    DISPLAY_HOURS_PATH="/run/rivet/display-hours"
    DISPLAY_OFF="19:00"
    DISPLAY_ON="07:00"
    {
        printf 'version = 1\n'
        printf 'hostname = "%s"\n' "$HOSTNAME_PROBE"
        sed -e 's/\r$//' -e '/^[[:space:]]*version[[:space:]]*=/d' "$NETWORK_DOC"
        printf '\n[control-plane]\n'
        printf 'listen = "%s"\n' "$CONTROL_LISTEN"
        # **`[display]` — ADR-039 §3.1, added 2026-09-02.**
        #
        # The predicate and both readers are unit-tested; what only a booted
        # machine can answer is whether the SUPERVISOR, converging a real
        # document on the target kernel, writes the file an application reads.
        # The window is arbitrary and never evaluated here — this asserts the
        # projection, not the arithmetic.
        printf '\n[display]\n'
        printf 'off = "%s"\n' "$DISPLAY_OFF"
        printf 'on = "%s"\n' "$DISPLAY_ON"
    } >"$DOC" || fail "could not compose $DOC"
    # A filter that stopped filtering would produce exactly the failure above,
    # and it took a whole run to read. One `version` line, counted.
    if [ "$(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC")" -ne 1 ]; then
        echo >&2
        cat -n "$DOC" >&2
        fail "the composed document has $(grep -c '^[[:space:]]*version[[:space:]]*=' "$DOC") \`version\` keys and TOML allows one. The fixture's own line is meant to be filtered out and was not — check its line endings."
    fi
    grep -q "^hostname = " "$DOC" \
        || fail "the hostname did not reach $DOC, so the naming assertion below would pass vacuously"
    grep -q "^listen = " "$DOC" \
        || fail "the [control-plane] listen did not reach $DOC, so the guest would bind $DEFAULT_LISTEN and the forwarded port would reach nothing"

    # **Nothing in this document enables the service**, and that is deliberate:
    # `cmd/rivet-sysd`'s definition is `enabled: true` and this boot is the only
    # place in the project that checks it. It said `false` for three whole
    # phases while every board ran a document that turned it on, so a card built
    # from the defaults booted with NO CONTROL PLANE and nothing could see it
    # (`PLAN.md` §94.7). A `[services.rivet-mcpd] enabled = true` here would
    # hide that again.
    if grep -q "$MCPD_NAME" "$DOC"; then
        fail "the harness document names $MCPD_NAME. It must not: what this boot asserts is that the IMAGE DEFAULT starts the control plane, and a document that turns it on would assert nothing."
    fi

    CP_LOG="$WORK/console.log"
    echo "smoke.sh: booting with that document, $MCPD_PATH, an identity at $MCPD_IDENTITY_DIR,"
    echo "smoke.sh: listening on $CONTROL_LISTEN and host port $CONTROL_PORT forwarded to it..."
    set -- "$INIT_BIN" \
        --supervisor "$SUP_BIN" --cli "$CLI_BIN" --netd "$NETD_BIN" \
        --mcpd "$MCPD_BIN" --secrets "$WORK/secrets"
    if [ -n "$NUSHELL_BIN" ]; then
        set -- "$@" --nushell "$NUSHELL_BIN"
    fi
    if [ -n "$TIMED_BIN" ]; then
        set -- "$@" --timed "$TIMED_BIN"
    fi
    "$HERE/run.sh" "$@" \
        --document "$DOC" --net --forward-port "$CONTROL_PORT:$CONTROL_PORT" \
        >"$CP_LOG" 2>&1 </dev/null &
    CP_QEMU=$!

    # Wait for the DAEMON's own line, not for a number of seconds. It is the
    # earliest moment a connection could be accepted, and it carries the address
    # the program actually bound — which is the fact the document was written to
    # change.
    #
    # **A supervisor that has GIVEN UP is an answer, and waiting the rest of
    # the timeout out for it would turn a diagnosis into a stopwatch.** That is
    # the failure this project has been bitten by before: `timeout`'s exit 124
    # says only that something did not happen, and every reason it did not
    # happen looks identical. `ADR-017` §3.5's give-up record is the supervisor
    # saying the machine is not going to recover, so it ends the wait
    # immediately and the block below reports what the daemon actually said.
    WAITED=0
    GAVE_UP=no
    while [ "$WAITED" -lt 120 ]; do
        if grep -qF 'rivet-mcpd: listening on' "$CP_LOG" 2>/dev/null; then
            break
        fi
        if grep -F "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" 2>/dev/null | grep -qF 'giving up'; then
            GAVE_UP=yes
            break
        fi
        # run.sh refuses a kernel with no virtio-net driver, and exits. Waiting
        # out the full timeout for a process that is already gone would report a
        # control plane failure for a reason that has nothing to do with one.
        if ! kill -0 "$CP_QEMU" 2>/dev/null; then
            cat "$CP_LOG" >&2
            fail "run.sh exited before the control plane reported listening — see its output above. The usual cause is a kernel with no virtio-net driver (run.sh refuses those by name), or host port $CONTROL_PORT already being in use."
        fi
        sleep 0.5
        WAITED=$((WAITED + 1))
    done
    if [ "$GAVE_UP" = yes ] || [ "$WAITED" -ge 120 ]; then
        kill "$CP_QEMU" 2>/dev/null || true
        echo >&2
        echo "--- what $MCPD_NAME itself said, on the guest ---" >&2
        grep -aF "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" | grep -aF '"stream":"stderr"' >&2 \
            || echo "      (it said nothing on stderr at all)" >&2
        echo "--- what the supervisor did about it ---" >&2
        grep -aF "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" | grep -avF '"stream":"stderr"' >&2 \
            || echo "      (not one line)" >&2
        if [ "$GAVE_UP" = yes ]; then
            fail "the control plane started, exited, and the supervisor GAVE UP on it.

    That is ADR-017 §3.5's threshold, and it is a diagnosis rather than a
    timeout: the daemon's own words are in the first block above. rivet-mcpd
    builds its TLS configuration and reads the document BEFORE it binds, and
    exits 2 for either — so a document it cannot parse, an identity it cannot
    read, and a pin file that is not there all end here, each saying which."
        fi
        fail "the control plane never reported listening within 60s of the boot, and the supervisor has not given up on it either. It is starting and neither binding nor exiting, which is neither of the two failures above."
    fi

    # --- the queries --------------------------------------------------------
    #
    # `cargo rivet run` composes each of these into an MCP `tools/call` from
    # `rivet_core::operation::CATALOGUE`, carries it over mutual TLS, and prints
    # the operation's own JSON unchanged. Nothing here reformats an answer.
    cp_run() {
        set +e
        "$CARGO_RIVET" run \
            --host "127.0.0.1:$CONTROL_PORT" \
            --identity "$WORK/operator" \
            --pin "$CP_PIN" \
            -- "$@" \
            >"$WORK/cp-out" 2>"$WORK/cp-err"
        CP_RC=$?
        set -e
    }
    CP_PIN=$GUEST_PIN

    # Retried, and **only for a failure to reach the machine at all**. The
    # daemon binds before `rivet-netd` has necessarily given eth0 its address,
    # so the first connection can arrive before there is anything at
    # 10.0.2.15 — a race this harness introduced and should absorb. A refusal
    # from the machine is an ANSWER and must be reported the first time it is
    # given, or a real regression takes three attempts to surface and looks
    # intermittent.
    ATTEMPT=1
    while :; do
        cp_run get status
        [ "$CP_RC" -eq 0 ] && break
        [ "$ATTEMPT" -ge 5 ] && break
        grep -qiE 'connection (refused|reset)|timed out|os error' "$WORK/cp-err" || break
        echo "smoke.sh: the guest is not reachable yet (attempt $ATTEMPT); retrying"
        ATTEMPT=$((ATTEMPT + 1))
        sleep 2
    done
    CP_STATUS_RC=$CP_RC
    CP_STATUS_OUT=$(tr -d '\r\n' <"$WORK/cp-out")
    CP_STATUS_ERR=$(tr -d '\r' <"$WORK/cp-err")

    if [ "$CP_STATUS_RC" -ne 0 ]; then
        echo >&2
        echo "--- what cargo rivet run said ---" >&2
        printf '%s\n' "${CP_STATUS_ERR:-(nothing)}" >&2
        echo "--- what the control plane said on the guest ---" >&2
        grep -aF 'rivet-mcpd:' "$CP_LOG" >&2 || echo "      (nothing)" >&2
        echo "--- what the supervisor said about the interface ---" >&2
        grep -aF '"svc":"rivet-netd"' "$CP_LOG" >&2 || echo "      (nothing)" >&2
        kill "$CP_QEMU" 2>/dev/null || true
        fail "\`cargo rivet run -- get status\` exited $CP_STATUS_RC.

    That is the whole of ADR-035's control plane in one command: mutual TLS with
    pinned Ed25519 keys, an MCP tools/call composed from the catalogue, and
    \`rivet mcp\` on the other end spawned by rivet-mcpd. A failure here is the
    handshake, the pin, the forwarded port, or the machine's own refusal — and the
    two blocks above say which."
    fi

    # `get catalogue`, which is what a caller asks when it does not yet know
    # what it may ask (`ADR-006` §8.13, ruled 2026-08-30). Asserted here because
    # this is the only place it has ever been asked over the wire it was built
    # for.
    cp_run get catalogue
    CP_CATALOGUE_RC=$CP_RC
    CP_CATALOGUE_OUT=$(tr -d '\r\n' <"$WORK/cp-out")

    # And one value read off the RUNNING machine rather than out of the document
    # that asked for it — `AGENTS.md` §6's difference between a setting being
    # configured and a setting working. `/etc/hosts` is the supervisor's own
    # projection and it carries the name the machine took.
    cp_run get file /etc/hosts
    CP_HOSTS_RC=$CP_RC
    CP_HOSTS_OUT=$(tr -d '\r' <"$WORK/cp-out")

    # --- the display window, ADR-039 §3.1 -----------------------------------
    #
    # **This is the only thing in this gate that a unit test could not have
    # said.** `rivet_core::display`'s predicate is asserted at every minute of
    # the day and `rivet_sdk::schedule` is asserted to agree with it — but both
    # run on a workstation against a string. Whether a document key becomes a
    # file on a booted machine is a question about the supervisor, the kernel
    # and the convergence order, and only a boot answers it.
    #
    # NOT in /etc, deliberately: that surface is for paths a C runtime demands
    # (`cmd/rivet-sysd/src/runtime.rs`), and nothing in libc reads a schedule.
    cp_run get file "$DISPLAY_HOURS_PATH"
    CP_DISPLAY_RC=$CP_RC
    CP_DISPLAY_OUT=$(tr -d '\r' <"$WORK/cp-out")
    if [ "$CP_DISPLAY_RC" -ne 0 ]; then
        echo >&2 "$CP_DISPLAY_OUT"
        cat >&2 "$WORK/cp-err"
        fail "the machine could not be asked for $DISPLAY_HOURS_PATH. The document carried a
    [display] table, so the supervisor should have written it (ADR-039 §3.1). A refusal here is
    either that file being absent — the projection did not run — or the control plane failing,
    which the queries above would already have caught."
    fi
    # The two times, in the order the file states them: `off` first. A reader
    # that took them the other way round would keep a building dark all day,
    # which is why the ORDER is asserted and not just the presence.
    case $CP_DISPLAY_OUT in
        *"$DISPLAY_OFF $DISPLAY_ON"*) : ;;
        *)
            echo >&2 "$CP_DISPLAY_OUT"
            fail "$DISPLAY_HOURS_PATH does not carry \`$DISPLAY_OFF $DISPLAY_ON\`. The document
    said off=$DISPLAY_OFF on=$DISPLAY_ON and the supervisor writes the pair on one line, off
    first (ADR-039 §3.1). Either the projection wrote something else or the format changed
    without rivet_sdk::schedule::Hours::parse changing with it."
            ;;
    esac
    # The provenance header, so a person who runs `cat` on it learns what to
    # change instead — ADR-007 §3.6.1, and the reason the format skips `#`.
    case $CP_DISPLAY_OUT in
        *"Generated by Rivet"*) : ;;
        *) fail "$DISPLAY_HOURS_PATH has no provenance header; a generated file that does not say
    what it is generated from is the interface ADR-007 §3.6.2 forbids" ;;
    esac
    echo "smoke.sh: the display window reached $DISPLAY_HOURS_PATH as \`$DISPLAY_OFF $DISPLAY_ON\`"

    # --- the journal ingest, and WHO asked — ADR-022 §11.8, ADR-028 §4.5 -----
    #
    # **Restored 2026-09-06.** `ADR-022` §11.8 claimed this was "a standing
    # assertion rather than a session somebody had once", and it had stopped
    # being one: the harness mode it rode on was removed with the old transport
    # and nothing replaced it, so for a while the ADR described a check that did
    # not exist. This is that check, over the control plane instead.
    #
    # **What it is for.** §11.3's whole argument is that TWO facts land on one
    # record and neither is redundant:
    #
    #   * `principal`  — what the CLI read out of its own environment. The
    #     pinned key the caller presented. SELF-REPORTED, and `ADR-028` §4.5 is
    #     careful that it is attribution and not authorisation.
    #   * `ingest_pid` — what the KERNEL said, from SO_PEERCRED, about the
    #     process on the other end of the ingest socket. Unforgeable by the
    #     sender.
    #
    # No version of `cmd/rivet` can write `ingest_pid`; the supervisor drops any
    # sender's attempt at it and substitutes the kernel's answer. **So its
    # presence is proof the record travelled the socket** rather than being
    # appended to the file by something that merely knew the format.
    #
    # **`-M virt` HAS NO `/state`, and that bounds what this can assert here.**
    # There is no `/dev/mmcblk0p5` under the emulator, so pid 1 reports
    # `DEGRADED /state not mounted` and `ADR-022` §8's files never exist. The
    # supervisor still ACCEPTS an ingested record — the socket is on `/run` —
    # but there is nowhere for it to land and `get log` reads files.
    #
    # So the two halves are asserted where each can be: **the mutating operation
    # and the ingest path here**, and **the record's two provenance fields on a
    # board**, where `PLAN.md` §148 and `ADR-028` §4.5.5 record the result. A
    # check that quietly passed on a guest with no journal would be worse than
    # no check, which is the mistake `ADR-022` §11.8 already made once.
    INGEST_PROBE=/state/ingest-probe.txt
    cp_run get storage
    CP_STORAGE_OUT=$(tr -d '\r\n' <"$WORK/cp-out")
    case $CP_STORAGE_OUT in
        *'"mount-point":"/state"'*) GUEST_HAS_STATE=yes ;;
        *) GUEST_HAS_STATE=no ;;
    esac
    if [ "$GUEST_HAS_STATE" = no ]; then
        echo "smoke.sh: this guest has no /state (-M virt has no /dev/mmcblk0p5), so the"
        echo "smoke.sh:   journal has no files and the ingest fields cannot be read back here."
        echo "smoke.sh:   ADR-022 §11.8 carries the board result instead. NOT asserted:"
        echo "smoke.sh:   ingest_pid, and the pinned principal on a journal record."
    else
    cp_run apply edit "$INGEST_PROBE" --append "smoke" confirm
    CP_INGEST_RC=$CP_RC
    CP_INGEST_OUT=$(tr -d '\r\n' <"$WORK/cp-out")
    if [ "$CP_INGEST_RC" -ne 0 ]; then
        echo >&2 "$CP_INGEST_OUT"
        cat >&2 "$WORK/cp-err"
        fail "\`apply edit $INGEST_PROBE\` was refused. /state is mounted and writable on this
    guest — the queries above read files off it — so this is the operation itself or the
    control plane's ability to carry a MUTATING one, which nothing before this point tests."
    fi
    # Asserted BEFORE the journal is read, because a record that was never
    # emitted and a journal that cannot be queried are different failures and
    # this is the one that says which.
    case $CP_INGEST_OUT in
        *'"journalled":true'*) : ;;
        *)
            echo >&2 "$CP_INGEST_OUT"
            fail "the edit did not report \`journalled: true\`, so \`cmd/rivet\` did not write a
    record at all and the two assertions below would be testing the wrong thing (ADR-006 §8.9.6)."
            ;;
    esac

    # `all` rather than the 200-record default: the probe's record is the most
    # recent thing on the machine, but a default that silently truncates is how
    # a real subject goes missing from a search.
    cp_run get log latest all
    CP_LOG_RC=$CP_RC
    CP_LOG_OUT=$(tr -d '\r' <"$WORK/cp-out")
    if [ "$CP_LOG_RC" -ne 0 ]; then
        cat >&2 "$WORK/cp-err"
        fail "\`get log\` was refused, so the journal could not be read back over the control
    plane (ADR-022 §9)."
    fi
    case $CP_LOG_OUT in
        *"$INGEST_PROBE"*) : ;;
        *) fail "the journal carries no record naming $INGEST_PROBE, so the edit reported
    \`journalled: true\` and nothing arrived — which is ADR-022 §11's ingest socket failing
    silently, the one failure the whole ingest design exists to make impossible." ;;
    esac
    # The kernel's half. Its presence is the proof of travel; the sender cannot
    # write this field.
    case $CP_LOG_OUT in
        *'"ingest_pid"'*) : ;;
        *) fail "the journal record carries no \`ingest_pid\`. That field is filled by the
    supervisor from SO_PEERCRED and cannot be written by any sender, so its absence means the
    record reached the file WITHOUT travelling the ingest socket — ADR-022 §11.3." ;;
    esac
    # The caller's half. `pinned-` is `rivet_core::deploy::Origin::ControlPlane`,
    # which is only reachable when `rivet-mcpd` put the peer's key into the
    # environment of the CLI it spawned — the half ADR-028 §4.5.5 records as
    # NEVER having been proved on a machine.
    case $CP_LOG_OUT in
        *'pinned-'*) : ;;
        *) fail "no journal record names a \`pinned-\` principal, so rivet-mcpd did not put
    RIVET_CONTROL_PLANE_PEER into the environment of the CLI it spawned and every mutating
    operation on this machine is recorded as having no caller at all (ADR-028 §4.5)." ;;
    esac
    echo "smoke.sh: the journal carries the probe, its ingest_pid and a pinned principal"
    fi

    # --- and the NEGATIVE, which is what makes the three above mean anything -
    #
    # A wrong pin must be refused. Without this, every assertion here would pass
    # equally well against a machine that authenticated nobody — and `ADR-025`
    # §2 makes the pin *the whole of the trust model*, so an exchange that
    # cannot be shown to fail is not evidence that it checked.
    #
    # The wrong pin is a real Ed25519 public key — the OPERATOR's own — rather
    # than 64 arbitrary characters, so what is exercised is the trust decision
    # and not the hex parser.
    CP_PIN=$OPERATOR_PIN
    cp_run get status
    CP_WRONG_PIN_RC=$CP_RC
    CP_WRONG_PIN_ERR=$(tr -d '\r' <"$WORK/cp-err")
    CP_WRONG_PIN_OUT=$(tr -d '\r\n' <"$WORK/cp-out")
    CP_PIN=$GUEST_PIN

    # --- THE STOP, and it is the last thing done to this guest ---------------
    #
    # `ADR-026` §4.5 got this far over the old channel and the ordering argument is
    # unchanged: `apply power-off` ends the run it is in the middle of, so it
    # goes after every other reading has been taken.
    #
    # **What is asserted is that the MACHINE STOPS**, not that a command exited
    # 0. pid 1 calls `reboot(RB_POWER_OFF)`, which on `-M virt` is a PSCI call
    # QEMU honours by exiting — so the process ending ON ITS OWN is the
    # observation, and an exit status cannot tell the two apart.
    echo "smoke.sh: asking the machine to power off, which ends this guest..."
    cp_run apply power-off confirm
    CP_STOP_RC=$CP_RC
    CP_STOP_ERR=$(tr -d '\r' <"$WORK/cp-err")
    WAITED=0
    while [ "$WAITED" -lt 40 ]; do
        if ! kill -0 "$CP_QEMU" 2>/dev/null; then
            STOPPED=yes
            break
        fi
        sleep 0.5
        WAITED=$((WAITED + 1))
    done
    # `|| true` because the guest may already be gone, and `kill` on a process
    # that has exited returns non-zero — which under `set -e` would end this run
    # between its work and its assertions, with no verdict at all.
    kill "$CP_QEMU" 2>/dev/null || true

    # --- what the DAEMON said about itself ----------------------------------
    #
    # Read off the console rather than inferred from the answer, because the two
    # can disagree in exactly one interesting way: a daemon that bound the
    # BUILT-IN DEFAULT while an operator's table said otherwise would still
    # answer here, on the loopback the tunnel happens to reach, and the
    # configuration would have been silently ignored. `PLAN.md` §82 is that
    # sentence being printed wrongly once already.
    if ! grep -qF "rivet-mcpd: listening on $CONTROL_LISTEN" "$CP_LOG"; then
        echo >&2
        grep -aF 'rivet-mcpd: listening on' "$CP_LOG" >&2 || true
        fail "the control plane did not report listening on $CONTROL_LISTEN, which is what the document's [control-plane] table asked for."
    fi
    if ! grep -F 'rivet-mcpd: listening on' "$CP_LOG" | grep -qF "from the document's [control-plane] table"; then
        echo >&2
        grep -aF 'rivet-mcpd: listening on' "$CP_LOG" >&2 || true
        fail "the control plane bound the right address and did NOT say it came from the document. That is PLAN.md §82's defect in the other direction: the flag records whether a table was READ, not whether one was given, and an operator's one question is whether their configuration took."
    fi
    # **Mutual TLS, said by the daemon.** Without this the whole exchange could
    # have run over the plaintext loopback transport and every assertion above
    # would be identical.
    if ! grep -qF 'rivet-mcpd: MUTUAL TLS' "$CP_LOG"; then
        fail "the control plane answered and never reported mutual TLS. ADR-035 §7.2 over ADR-025 §2 is what makes a non-loopback bind legal at all, so a machine answering without it is one that would accept anybody."
    fi
    # And the SUPERVISOR started it, which is the half `run.sh` cannot fake:
    # nothing in this harness execs the daemon.
    if ! grep -F "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" | grep -qF '"msg":"started"'; then
        echo >&2
        grep -aF "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" >&2 || echo "      (nothing)" >&2
        fail "the control plane answered and the supervisor never reported starting it. Nothing in this harness execs that binary, so either the record was lost or something else is serving that port."
    fi
    if grep -F "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" | grep -qF 'REFUSED this image cannot run'; then
        fail "the supervisor refused to start the control plane in a boot that packed it — run.sh put the binary somewhere the definition does not look: $(grep -aF "\"svc\":\"$MCPD_NAME\"" "$CP_LOG" | head -1)"
    fi

    # --- the answers, asserted ----------------------------------------------
    #
    # The supervisor answered out of its own memory, over ADR-032 §3.2's query
    # socket, and the CLI parsed it back — then rivet-mcpd carried it out. Two
    # sockets and a TLS session between the question and the answer.
    case $CP_STATUS_OUT in
        *'"phase":"running"'*) ;;
        *) fail "the status did not report a running supervisor: $CP_STATUS_OUT" ;;
    esac
    # **And the control plane is in it, started, with a pid** — which is the
    # proof the answer is live rather than a shape: this question arrived
    # THROUGH that service, so it cannot be anything but started.
    case $CP_STATUS_OUT in
        *"\"name\":\"$MCPD_NAME\""*) ;;
        *) fail "the status names no $MCPD_NAME, and this question arrived over it: $CP_STATUS_OUT" ;;
    esac
    case $CP_STATUS_OUT in
        *'"state":"started"'*) ;;
        *) fail "no service in the status is started, on a machine that is answering: $CP_STATUS_OUT" ;;
    esac

    if [ "${CP_CATALOGUE_RC:-1}" -ne 0 ]; then
        fail "\`cargo rivet run -- get catalogue\` exited ${CP_CATALOGUE_RC:-?} on a machine that had just answered \`get status\`"
    fi
    case $CP_CATALOGUE_OUT in
        *'"verbs":['*) ;;
        *) fail "get catalogue returned no verb list: $CP_CATALOGUE_OUT" ;;
    esac
    # **The whole vocabulary, not only what is built.** `ADR-006` §8.13's third
    # point: a caller reading `tools/list` sees three verbs and concludes the
    # vocabulary is three. The catalogue must publish the unimplemented ones
    # too, or it is making a claim of absence it cannot support.
    case $CP_CATALOGUE_OUT in
        *'"implemented":false'*) ;;
        *) fail "get catalogue published no unimplemented verb. ADR-006 §6.4's vocabulary is nine words and this build has an operation behind three; a catalogue that lists only the three tells a caller the other six do not exist: $CP_CATALOGUE_OUT" ;;
    esac

    if [ "${CP_HOSTS_RC:-1}" -ne 0 ]; then
        fail "\`cargo rivet run -- get file /etc/hosts\` exited ${CP_HOSTS_RC:-?} on a machine that had just answered twice"
    fi
    case $CP_HOSTS_OUT in
        *'"content"'*) ;;
        *) fail "get file /etc/hosts came back with no content field, so nothing was read off the running machine: $CP_HOSTS_OUT" ;;
    esac
    # The name the machine actually took, read off a file the supervisor
    # projected — not off the document that asked for it.
    case $CP_HOSTS_OUT in
        *"$HOSTNAME_PROBE"*) ;;
        *) fail "the document asked for hostname=$HOSTNAME_PROBE and /etc/hosts does not carry it, so this machine reports a name it cannot resolve. rivet_core::etc::render_hosts is where the line comes from: $CP_HOSTS_OUT" ;;
    esac

    # --- the wrong pin, asserted --------------------------------------------
    if [ "${CP_WRONG_PIN_RC:-0}" -eq 0 ]; then
        fail "a connection pinned to the WRONG key exited 0 and was answered: $CP_WRONG_PIN_OUT

    ADR-025 §2 makes the pin the whole of the trust model, so this machine
    authenticated a certificate it was told not to trust — and every assertion
    above would pass equally well against a control plane that checked nobody."
    fi
    if [ -n "$CP_WRONG_PIN_OUT" ]; then
        fail "a connection pinned to the wrong key was refused AND produced an answer: $CP_WRONG_PIN_OUT"
    fi

    # --- the stop, asserted -------------------------------------------------
    if [ "$STOPPED" != yes ]; then
        echo >&2
        echo "--- what the stop said ---" >&2
        printf '%s\n' "${CP_STOP_ERR:-(nothing)}" >&2
        echo "--- what pid 1 said on its way down ---" >&2
        grep -aiE 'power|shutdown|stopping|reboot' "$CP_LOG" | tail -20 >&2 || true
        fail "\`apply power-off confirm\` over the control plane did not stop the machine within 20s. The command's exit status was ${CP_STOP_RC:-?}, and this assertion is deliberately about the GUEST GOING AWAY rather than about that: pid 1 calls reboot(RB_POWER_OFF), which -M virt turns into a PSCI call QEMU honours by exiting. A machine that accepted the command and stayed up is the failure worth catching."
    fi
fi

echo
if [ -n "$CLI_BIN" ]; then
    echo "smoke.sh: PASS (PID 1, the supervisor it spawned, and a CLI at $CLI_PATH)"
else
    echo "smoke.sh: PASS (PID 1 and the supervisor it spawned, with no CLI packed)"
fi
echo "  - kernel reached 'Run /init as init process'"
if [ -n "$MCPD_BIN" ]; then
    echo "  - BOOT 2 ANSWERED, over ADR-035's control plane and nothing else:"
    echo "    a throwaway identity was minted, placed at $MCPD_IDENTITY_DIR,"
    echo "    the supervisor started $MCPD_NAME from the IMAGE DEFAULT, and"
    echo "    \`cargo rivet run\` carried three operations over mutual TLS —"
    echo "      get status     $(printf '%s' "$CP_STATUS_OUT" | cut -c1-96)"
    echo "      get catalogue  $(printf '%s' "$CP_CATALOGUE_OUT" | cut -c1-96)"
    printf '      get file      %s\n' \
        "$(printf '%s' "$CP_HOSTS_OUT" | tr -d '\n' | cut -c1-96)"
    echo "    and a connection pinned to the WRONG key was refused before any of"
    echo "    it could happen, which is what makes the three above mean anything:"
    printf '%s\n' "${CP_WRONG_PIN_ERR:-(nothing)}" | head -2 | sed 's/^/      /'
    if [ "$STOPPED" = yes ]; then
        echo "  - \`apply power-off confirm\` STOPPED the machine, over that same"
        echo "    control plane: pid 1 took the signal and the guest exited on its"
        echo "    own (ADR-017 3.1.5, ADR-026 §4.5)"
    fi
fi
echo "  - console showed 'rivet-initd: starting as pid 1'"
echo "  - no 'Kernel panic' in the console output"
echo "  - QEMU was still running (PID 1 parked) when the ${TIMEOUT_SECS}s timeout stopped it"
echo "  - PID 1 spawned $SUPERVISOR_PATH and reported its pid"
echo "  - the supervisor announced itself on the same console"
echo "  - the supervisor never exited, so PID 1 never retried, backed off or gave up"
echo "  - the supervisor read the cgroup2 hierarchy, said so ONCE for the machine,"
echo "    and said NOTHING about any service's own cgroup (ADR-017 §3.1.3.1):"
grep -F '"msg":"the cgroup2 hierarchy is readable' "$LOG" \
    | head -1 | sed -n 's/.*\("hierarchy":"[^"]*"\).*/      \1/p'
echo "  - the supervisor rendered all $EXPECTED_ETC files of /etc, and each read back byte-for-byte:"
grep -F '"msg":"rendered"' "$LOG" \
    | sed -n 's/.*"path":"\([^"]*\)".*"mode":"\([^"]*\)".*"bytes":\([0-9]*\).*/      \1  mode \2  \3 bytes/p'
echo "  - and the kernel's own mount table says that filesystem is read-only:"
grep -F '"kernel_says_read_only":true' "$LOG" \
    | head -1 | sed -n 's/.*"line":"\([^"]*\)".*/      \1/p'
echo "  - the journal opened on /state and flushed what it had buffered before"
echo "    that (ADR-022 §7), so this boot is on a file rather than only here:"
grep -F '"msg":"the journal is on the card' "$LOG" \
    | head -1 | sed -n 's/.*\("path":"[^"]*".*"flushed":[0-9]*\).*/      \1/p'
echo "  - PID 1 reached a boot-confirmation verdict instead of withholding one:"
grep -E "rivet-initd: (boot confirmed:|boot could NOT be confirmed:|not a tryboot boot|partition [0-9]+ is already the default)" "$LOG" \
    | head -1 | sed 's/^/      /'
if [ -n "$CLI_BIN" ]; then
    echo "  - the projection PROBED LOGIN_SHELL_PATHS, found what was packed,"
    echo "    and wrote an /etc/shells naming it — the probe's other answer."
    echo "    'shell' is root's, the first present candidate; 'listed' is how"
    echo "    many /etc/shells names (the dual-login-shell ruling):"
    grep -F '"msg":"/etc/shells names a login shell' "$LOG" \
        | head -1 | sed -n 's/.*\("shell":"[^"]*".*"listed":[0-9]*\).*/      \1/p'
else
    echo "  - the projection PROBED LOGIN_SHELL_PATHS, found nothing, and refused to"
    echo "    name a shell that is not there rather than writing one anyway:"
    grep -F '"msg":"/etc/shells names NO login shell' "$LOG" \
        | head -1 | sed -n 's/.*\("would_be":"[^"]*"\).*/      \1/p'
fi
echo "  - BOOT 1 had NO route in, and said so once rather than silently: the"
echo "    supervisor's $MCPD_NAME definition is enabled, the binary was not in"
echo "    this image, and the spawn refused permanently naming what was lost"
echo "    (ADR-017 §3.1.6). That is what a card without it really does:"
grep -F "\"svc\":\"$MCPD_NAME\"" "$LOG" | grep -F 'REFUSED this image cannot run' \
    | head -1 | sed -n 's/.*\("program":"[^"]*".*"errno":[0-9]*\).*/      \1/p'
echo
echo "  Still NOT settled by any of the above:"
echo
echo "  - Whether the boot can actually be CONFIRMED. That needs"
echo "    /proc/device-tree/chosen/bootloader/tryboot, which ADR-026 §4 says is"
echo "    not emulated at all. Only the board settles it."
echo
if [ -n "$MCPD_BIN" ]; then
    echo "  - Whether anything can ASK THIS MACHINE A QUESTION. **SETTLED, in the"
    echo "    emulator**, by BOOT 2 — and it had not been since 2026-08-29, when"
    echo "    ADR-035 left the control plane as the only way in, and this gate"
    echo "    could boot a Rivet and not speak to it (PLAN.md §102.5)."
    echo
    echo "    What ran: two Ed25519 identities minted by \`cargo rivet identity"
    echo "    new\`, the guest's placed at $MCPD_IDENTITY_DIR with a peers file"
    echo "    naming the operator's; a document whose [control-plane] listen is"
    echo "    $CONTROL_LISTEN, because SLIRP cannot route to the built-in"
    echo "    $DEFAULT_LISTEN; the supervisor's own definition starting the"
    echo "    daemon with NO document key enabling it; and \`cargo rivet run\` —"
    echo "    the binary an operator holds — composing each command into an MCP"
    echo "    tools/call from the catalogue and carrying it over mutual TLS to a"
    echo "    \`rivet mcp\` that rivet-mcpd spawned."
    echo
    echo "    The daemon's own account of what it did, off the guest console:"
    grep -aF 'rivet-mcpd: listening on' "$CP_LOG" | head -1 | sed 's/^/      /'
    grep -aF 'rivet-mcpd: MUTUAL TLS' "$CP_LOG" | head -1 | sed 's/^/      /'
    echo
    echo "    What that does NOT settle: anything on a BOARD; enrollment (the"
    echo "    identity arrived through --secrets, standing in for cmd/rivet-image"
    echo "    writing it onto p5 at provisioning — ADR-025 §3); the first-boot"
    echo "    path (the document arrived through --document, standing in for PID 1"
    echo "    reading the boot partition, which -M virt does not have — ADR-004"
    echo "    §6); and anything about the board's own NIC, because this is virtio."
    echo
    echo "  - The line below is the projection's verdict on the file:"
elif [ -n "$CLI_BIN" ]; then
    echo "  - Whether anything can ASK THIS MACHINE A QUESTION. **NOT settled by"
    echo "    THIS run**, and the reason is the run rather than the system: no"
    echo "    control plane was packed, so the supervisor refused to start one and"
    echo "    nothing can reach this machine at all. Pass a fourth binary — or no"
    echo "    arguments, and the build produces it — for BOOT 2, which mints an"
    echo "    identity, forwards $CONTROL_PORT and runs three operations over it."
    echo "    The line below is the projection's verdict on the file, not on a"
    echo "    connection:"
else
    echo "  - Whether anything can ASK THIS MACHINE A QUESTION. It cannot, twice"
    echo "    over: no control plane was packed, and no cmd/rivet either — and"
    echo "    rivet-mcpd hands each caller the stdin and stdout of that binary, so"
    echo "    even a running daemon would accept a connection and have nothing to"
    echo "    spawn. This is the deliberate negative direction. The console line"
    echo "    below is about the login-shell projection and says so:"
fi
# No `$` anchor anywhere in this file's sed: the guest console is a serial tty,
# so every line arrives CRLF-terminated and `}$` matches nothing.
grep -F '"msg":"/etc projected"' "$LOG" \
    | head -1 | sed -n 's/.*\("outcome":"[^"]*","permits_a_login":[a-z]*\).*/      \1/p'
echo
echo "  - Whether a hand edit is actually REFUSED. The mount carries 'ro' by the"
echo "    kernel's own account, which is the mechanism ADR-007 §3.6.1 item 1"
echo "    requires — but nothing here attempts a write to /etc and watches it"
echo "    fail."
if [ -n "$NUSHELL_BIN" ]; then
    echo "    A machine with nu on it CAN attempt one — which is new, and is the"
    echo "    reason this paragraph was rewritten on 2026-08-24 rather than left"
    echo "    saying 'there is no shell on the machine' (AGENTS.md §7). Nothing"
    echo "    in this harness does: the run above execs nu only for the two"
    echo "    non-interactive commands it asserts on, and neither writes to /etc."
else
    echo "    There is no shell on this machine to attempt one with. Packing the"
    echo "    CLI does not change that: nothing execs it, it has no interactive"
    echo "    prompt by ruling (ADR-005 §5.1), and it executes no programs"
    echo "    (cmd/rivet's invocation module). RIVET_NUSHELL puts a real"
    echo "    shell on the machine — it still does not attempt the write."
fi
echo
echo "  - Anything at all about hardware. This is an initramfs on QEMU: the"
echo "    /etc mount point is created here, whereas on a board it comes out of"
echo "    the read-only erofs image (ADR-018 §3), which is a different code path"
echo "    through mkdir_p."
echo
echo "  - That the JOURNAL survives anything, which is the half its name"
echo "    suggests and the half this cannot reach. /state here is a directory"
echo "    on the initramfs, so it is writable — which is why the write path"
echo "    above could be asserted at all — and it is RAM, so it vanishes with"
echo "    the guest. ADR-022 §8's rotation is untouched too: a boot writes a"
echo "    few kilobytes against a 1 MiB cap."
echo
echo "    NARROWED 2026-08-28 (PLAN.md §62): 'rivet get log previous' HAS now"
echo "    selected a boot that actually ended — run.sh --card --card-rw booted"
echo "    twice keeps the first boot's journal on real ext4. This gate does not"
echo "    do that, so the sentence still describes THIS RUN. It is no longer a"
echo "    statement about the loop."
echo
echo "  - That the control plane's identity, or anything else on /state, is on"
echo "    ext4 IN THIS RUN. This gate boots from an initramfs with no disk"
echo "    attached, so PID 1's mount refuses and device.seed, device.crt and the"
echo "    peers file all sit on the INITRAMFS tmpfs."
echo
echo "    ~~The second-boot half of idempotence is unreachable in this loop.~~"
echo "    STRUCK 2026-08-28 — PLAN.md §62 and §65. run.sh --card attaches a card"
echo "    image as virtio-blk, tools/qemu/seed-card.sh puts a document on its"
echo "    /state, and three boots of one card settled that a generated file is"
echo "    made once and converges on the next boot. What is unreachable is not"
echo "    the QUESTION, it is this GATE, which has no --card option yet."
echo
echo "  - Anything about a login. ADR-035 left the control plane as the only"
echo "    way in; this gate no longer packs a login server or logs in, and the"
echo "    stage that did was deleted 2026-08-30 along with ~200 lines."
exit 0
