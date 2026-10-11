#!/bin/sh
# tools/qemu/adr-051-proof.sh — **the supervisor really derives the backup
# table, on a booted machine, from a signed manifest.**
#
# `ADR-051` §5. Run it in the container:
#
#   RIVET_CACHE_TAG=-backup tools/container/exec.sh tools/qemu/adr-051-proof.sh
#
# ## Why this exists as a script rather than a stage in `smoke.sh`
#
# `tools/qemu/adr-042-proof.sh`'s reason, and the same shape. `smoke.sh` is the
# gate and its two boots are about PID 1, `/etc`, the journal and the control
# plane; this asks one narrow question that needs its own document, its own
# installed application and two boots of its own. It is a script so that it can
# be RE-RUN — `AGENTS.md` §4: a test suite that cannot be run will rot silently.
#
# ## Two boots, because one alone proves nothing
#
# The two differ by **one manifest** and by nothing else. The same binary, the
# same document, the same fixture key.
#
# * **Boot 1** installs an application whose manifest declares `proxy-path`,
#   `backup-path` and `backup-format`. The supervisor must publish **one**
#   declaration.
# * **Boot 2** installs the same application with the backup keys removed. The
#   supervisor must publish **zero** — and must still publish, because
#   `ADR-051` §5.2 makes an empty table the answer *nothing is offered* rather
#   than an absent file meaning *this build does not answer that question*.
#
# **A run that only asserted the first would pass against a supervisor that
# wrote `declarations 1` unconditionally**, which is exactly the class of defect
# `ADR-045` shipped: a vocabulary nothing read, that looked built.
#
# ## What it does NOT settle
#
# It reads the supervisor's own journal line rather than the file's bytes,
# because nothing on this guest can read `/run/rivet/backups` back: an
# application is confined by `ADR-042`'s ruleset and was never granted that
# path, and the control plane is not brought up here. So what is proved is that
# `publish_backups` ran, read a real installed manifest, derived the right
# number of rows, and that `write_file` returned `Ok` — the failure path emits a
# different line, which this asserts is absent. **The file's CONTENT, and the
# transport that reads it and fetches what it names, are
# `tools/qemu/adr-051-restore-proof.sh`'s**: that guest runs the control plane,
# and `cargo rivet backup` reads the table and fetches each payload over it
# (`ADR-051` §8.1, `.rulings/` R-234). This script stays the two-boot control
# for the derivation alone.
#
# `ADR-026` §4's right-hand column is untouched, entirely.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO/target` — `tools/container/exec.sh` mounts a
# named volume there and exports it, so a script that composed the in-tree path
# would build correctly and then look where nothing put the binaries.
OUT=${CARGO_TARGET_DIR:-$REPO/target}/$TARGET/release

WORK=${TMPDIR:-/tmp}/adr-051-proof
rm -rf "$WORK"
mkdir -p "$WORK"

echo "adr-051-proof: building the supervisor, pid 1 and the probe for $TARGET"
cargo build --release --target "$TARGET" -p rivetos-initd -p rivetos-sysd
( cd "$HERE/testapp" && cargo build --release --target "$TARGET" )
PROBE=$OUT/rivet-testapp
if [ ! -f "$PROBE" ]; then
    echo "adr-051-proof: the probe was not built at $PROBE" >&2
    exit 1
fi

# **Installed under a name with none of `tools/qemu/testapp`'s mode words in
# it**, so it takes that fixture's default behaviour — print a pid and sleep.
# Nothing about this proof depends on what the application DOES; it depends on
# what its manifest SAYS.
NAME=backup-probe
INSTALLED=$WORK/$NAME
cp "$PROBE" "$INSTALLED"

# The document that enables it. One stanza, and it is the whole of what an
# operator contributes here — `ADR-051` §5's claim is that no column of the
# table comes from a document, and this file is the control for it.
printf 'version = 1\n\n[services.%s]\nenabled = true\n' "$NAME" >"$WORK/document.toml"

# **Two manifests, differing in three lines.** Written as the canonical text
# `Manifest::render` emits, in `KEYS` order, because that is what an installer
# leaves on disk and what the supervisor parses.
common='kind = "daemon"
restart = "never"
ready = "none"
network = "none"
state = "private"
listen = ["tcp:8080"]
provides = []
connects = []
library = []
devices = []
proxy-path = "probe"
'
printf '%s' "$common" >"$WORK/offers.toml"
printf 'backup-path = "api/v1/backup"\nbackup-format = "sqlite"\n' >>"$WORK/offers.toml"
printf '%s' "$common" >"$WORK/offers-nothing.toml"

( cd "$HERE/fixture-signer" && cargo build --release )
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer

if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

SECONDS_TO_RUN=${RIVET_PROOF_SECONDS:-45}

# One boot, with one manifest. Prints the supervisor's line and returns the
# number of declarations it reported.
boot() {
    manifest=$1
    log=$2
    "$SIGNER" "$NAME" "$INSTALLED" "$manifest" "$WORK/header" "$WORK/trust" >/dev/null
    set +e
    timeout -k 2 "$SECONDS_TO_RUN" "$HERE/run.sh" \
        "$OUT/rivet-initd" \
        --supervisor "$OUT/rivet-sysd" \
        --document "$WORK/document.toml" \
        --app "$NAME:$INSTALLED:$manifest:$WORK/header" \
        --trust "$WORK/trust" \
        >"$log" 2>&1 </dev/null
    set -e
}

echo "adr-051-proof: boot 1 — a manifest that OFFERS, bounded at ${SECONDS_TO_RUN}s"
boot "$WORK/offers.toml" "$WORK/offers.log"
echo "adr-051-proof: boot 2 — the same manifest with the backup keys removed"
boot "$WORK/offers-nothing.toml" "$WORK/offers-nothing.log"

verdict=0
check() {
    log=$1
    want=$2
    what=$3
    echo "--- $what ---"
    line=$(grep -ao 'what this machine.s applications offer to back up[^}]*}' "$log" | head -1 || true)
    if [ -z "$line" ]; then
        echo "adr-051-proof: FAIL — the supervisor published NOTHING on this boot." >&2
        echo "    publish_backups is called from cmd/rivet-sysd/src/main.rs beside" >&2
        echo "    publish_routes; a boot with no line at all did not reach it." >&2
        verdict=1
        return
    fi
    echo "  $line"
    if grep -aq 'could not be published' "$log"; then
        echo "adr-051-proof: FAIL — the write failed; the table was not written" >&2
        verdict=1
        return
    fi
    case $line in
        *'"declarations":'"$want"*) echo "  OK — $want declaration(s), as $what requires" ;;
        *)
            echo "adr-051-proof: FAIL — expected $want declaration(s) for $what" >&2
            verdict=1
            ;;
    esac
}

check "$WORK/offers.log" 1 "a manifest declaring backup-path and backup-format"
check "$WORK/offers-nothing.log" 0 "the same manifest with them removed"

# **The control.** Both boots must have got as far as reading that manifest at
# all, or the counts above are two readings of a machine that never looked.
# `publish_routes` runs immediately before `publish_backups` from the same list,
# so its line is the evidence that the application was discovered.
echo "--- the control: the application was discovered on both boots ---"
for log in "$WORK/offers.log" "$WORK/offers-nothing.log"; do
    if grep -aq "$NAME" "$log"; then
        echo "  $(basename "$log"): the supervisor names $NAME"
    else
        echo "adr-051-proof: FAIL — $NAME appears nowhere in $log, so nothing on that" >&2
        echo "    boot demonstrates the manifest was read (AGENTS.md §7)" >&2
        verdict=1
    fi
done

if [ "$verdict" -eq 0 ]; then
    echo "adr-051-proof: PASS — both directions"
else
    echo "adr-051-proof: FAIL — logs are in $WORK" >&2
fi
exit "$verdict"
