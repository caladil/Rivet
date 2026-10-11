#!/bin/sh
# Rehearse the whole certificate-authority ceremony from the COMMAND LINE, and
# assert what it produced — `PLAN.md` §146.1.
#
# Usage:
#   tools/provision/rehearse.sh                 # default fleet and suffixes
#   tools/provision/rehearse.sh <fleet> <suffixes> <hostname>
#
#   e.g. tools/provision/rehearse.sh home-fleet local,home-fleet.home.arpa,home-fleet.lan arbor.local
#
# ## Why this exists, and what it is NOT
#
# **It is not a way to provision a fleet.** The maintainer provisions his own,
# at the keyboard, through the interactive path, deliberately experiencing what a
# newcomer experiences. Nothing here is meant to save him a step, and a change
# that started to would be the wrong turn.
#
# **It is a REGRESSION HARNESS for the ceremony.** `ADR-014`'s ports and
# adapters put the decisions in the domain and make the command line and the
# menu two adapters over one core — so exhausting the ceremony HERE proves the
# machinery, and leaves his walkthrough to test the interface, which is the part
# only a person can judge. Every defect this finds is one he does not meet.
#
# ## It asserts the ARTIFACT, not the exit status
#
# `PLAN.md` §151 and §153 are two days of evidence that a thing can run, report
# success, and produce the wrong result: an `explain` that answered for 26 paths
# when the tables held 19, a load-bearing list of four that was two, a deploy
# that said it armed a trial boot and did not. **A ceremony that completed is
# not a ceremony that produced the right certificate**, so every step below
# checks what landed on disk.
#
# The suffix list is the sharpest case. `PLAN.md` §149.4 measured that a
# cross-signed root INHERITS the old root's name constraint, so the list is
# frozen at mint and cannot be widened afterwards without touching every device.
# A root minted with a suffix missing is not a thing you notice later and fix.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

FLEET=${1:-rehearsal-fleet}
SUFFIXES=${2:-local,rehearsal-fleet.home.arpa}
HOSTNAME_=${3:-arbor.local}

# **A throwaway directory, never `~/.rivet/ca`.** A rehearsal that wrote where
# production lives would leave debris the real mint inherits, and nobody would
# know which run produced which file.
WORK=${RIVET_REHEARSE_DIR:-$(mktemp -d 2>/dev/null || echo "/tmp/rivet-rehearse.$$")}
CA="$WORK/ca"

FAILED=""
fail() {
    echo "  FAIL: $1" >&2
    FAILED="yes"
}
ok() { echo "  ok   $1"; }

cleanup() {
    # Kept on failure, deliberately: the artifact is the evidence.
    if [ -z "$FAILED" ] && [ -z "${RIVET_REHEARSE_KEEP:-}" ]; then
        rm -rf "$WORK"
    else
        echo ""
        echo "  the artifacts are in $WORK"
    fi
}
trap cleanup EXIT

echo "rehearse.sh: fleet=$FLEET suffixes=$SUFFIXES host=$HOSTNAME_"
echo "rehearse.sh: working in $WORK"
echo ""

# --- the tool -----------------------------------------------------------------
#
# Built from THIS tree and called by path. `PLAN.md` §2 records that the
# `cargo-rivet` on a PATH is routinely stale and refuses valid input; the same
# hazard applies to anything installed.
echo "rehearse.sh: building rivet-provision from this tree..."
( cd "$REPO" && cargo build --release -p rivetos-provision >/dev/null 2>&1 ) \
    || { echo "  FAIL: rivet-provision does not build" >&2; exit 1; }
# **`CARGO_TARGET_DIR` first.** The container sets it to `/target`, so
# `$REPO/target` does not exist inside it and this script could only ever run on
# Windows — which it silently did, reporting "no rivet-provision binary" after a
# build that had just succeeded. Found 2026-09-07 by `rehearse-menu.sh`, which
# copied the line.
TARGET=${CARGO_TARGET_DIR:-$REPO/target}
PROVISION="$TARGET/release/rivet-provision"
[ -x "$PROVISION" ] || PROVISION="$TARGET/release/rivet-provision.exe"
[ -x "$PROVISION" ] || { echo "  FAIL: no rivet-provision binary under $TARGET/release" >&2; exit 1; }

have_openssl=no
command -v openssl >/dev/null 2>&1 && have_openssl=yes
[ "$have_openssl" = yes ] || echo "  NOT CHECKED: certificate contents — no openssl on this host."

# --- 1. mint the authority ----------------------------------------------------
echo ""
echo "--- 1. mint the authority"
mkdir -p "$CA"
# **`--no-install`, and it is a safety flag rather than a tidiness one.** The
# `ca` verb files the operator's identity into the Windows Personal store with
# `certutil`, which Windows resolves from the system directory — so a rehearsal
# cannot be made harmless by adjusting `PATH`. Every run of this script used to
# leave a `CN=Rehearsal Operator` certificate in the real store.
"$PROVISION" ca "$SUFFIXES" "$FLEET" "$CA" "Rehearsal Operator" --no-install >"$WORK/ca.log" 2>&1 \
    || { cat "$WORK/ca.log" >&2; fail "the mint itself was refused"; exit 1; }

grep -q "filed into" "$WORK/ca.log" \
    && fail "the rehearsal INSTALLED a certificate into a store — --no-install did not take" \
    || ok "nothing was filed into a certificate store"

for f in root.crt root.key suffix; do
    [ -s "$CA/$f" ] && ok "$f written" || fail "$f is missing or empty"
done

# **Every suffix reached the constraint.** This is the assertion the whole
# script exists for: §149.4 makes the list unwidenable, so a suffix that did not
# land is a root that has to be re-minted and re-trusted everywhere.
if [ "$have_openssl" = yes ]; then
    openssl x509 -in "$CA/root.crt" -noout -text >"$WORK/root.txt" 2>/dev/null || true
    echo "$SUFFIXES" | tr ',' '\n' | while read -r s; do
        [ -n "$s" ] || continue
        if grep -qF "DNS:$s" "$WORK/root.txt"; then
            echo "  ok   constraint permits $s"
        else
            echo "  FAIL: the root does NOT permit $s — and a name constraint cannot be widened later (PLAN.md §149.4)" >&2
            echo "fail" > "$WORK/.failed"
        fi
    done
    [ -f "$WORK/.failed" ] && FAILED=yes

    # A root that is not a CA, or that constrains nothing, is the failure
    # `cmd/rivet-provision`'s own module documentation exists to prevent.
    grep -q "CA:TRUE" "$WORK/root.txt" && ok "the root is a CA" || fail "the root is not a CA"
    grep -q "X509v3 Name Constraints" "$WORK/root.txt" \
        && ok "the root carries name constraints" \
        || fail "the root constrains NOTHING — it could vouch for any site on the internet"
fi

# The suffix note must agree with what was asked for. **Nothing reads this file
# any more** — Q13, 2026-09-06, moved both mint paths onto the certificate's own
# name constraint — but it is still written, and a note a person might `cat` and
# believe is worth checking against the thing it describes.
noted=$(tr '\n' ',' < "$CA/suffix" | sed 's/,$//')
[ "$noted" = "$(echo "$SUFFIXES" | tr -d ' ')" ] \
    && ok "the suffix note matches what was minted" \
    || fail "the suffix note says '$noted' and the mint was given '$SUFFIXES'"

# --- 2. issue the fleet server's leaf -----------------------------------------
#
# **`ca server` COMPOSES from the authority by default**, and `--names` is the
# literal list. Before the list existed at all it issued for the one string
# typed, so `ca server arbor` produced a certificate whose only subject
# alternative name was `DNS:arbor` — a bare label no browser matches — while the
# menu composed every permitted name. Two adapters, two certificates, each
# internally consistent: `PLAN.md` §156.2.
#
# **This section drives the literal path on purpose.** The composing path is
# exercised in §2a below, and the two are asserted to agree — which is a
# stronger claim than either alone.
echo ""
echo "--- 2. issue the server certificate"
host_label=${HOSTNAME_%%.*}
WANT=$(echo "$SUFFIXES" | tr ',' '\n' | while read -r s; do
    [ -n "$s" ] || continue
    printf '%s.%s\n' "$host_label" "$s"
done | tr '\n' ',' | sed 's/,$//')
echo "  asking for: $WANT"

"$PROVISION" ca server --names "$WANT" "$CA" >"$WORK/server.log" 2>&1 \
    || { cat "$WORK/server.log" >&2; fail "the server certificate was refused"; }

# --- 2a. the composing form reaches the same list, from a bare hostname --------
#
# `ca server rivet-01.local` once took that literally and replaced a three-name
# leaf with a one-name one on a production fleet server; only the first name then
# validated. Both spellings a person reaches for are asserted here.
for typed in "$host_label" "$host_label.$(echo "$SUFFIXES" | cut -d, -f1)"; do
    "$PROVISION" ca server "$typed" "$CA" >"$WORK/composed.log" 2>&1 \
        || { cat "$WORK/composed.log" >&2; fail "ca server $typed was refused"; continue; }
    got=$(sed -n 's/^  minting for //p' "$WORK/composed.log" | tr -d ' ')
    [ "$got" = "$(echo "$WANT" | tr -d ' ')" ] \
        && ok "ca server $typed composed $got" \
        || fail "ca server $typed composed '$got' and the list is '$WANT'"
done

leaf=$(ls "$CA"/server-*.crt 2>/dev/null | head -1 || true)
[ -n "$leaf" ] && ok "server leaf written ($(basename "$leaf"))" || fail "no server leaf was written"

# **One certificate carrying EVERY name, and this is a FAILURE when it is not.**
# It used to print a note and pass. An authority permitted three suffixes whose
# leaf carries one means mDNS works and DNS does not — or the reverse — with
# nothing on screen saying why, which is the shape of defect this whole script
# exists to catch rather than describe.
if [ "$have_openssl" = yes ] && [ -n "$leaf" ]; then
    openssl x509 -in "$leaf" -noout -text >"$WORK/leaf.txt" 2>/dev/null || true
    echo "$WANT" | tr ',' '\n' | while read -r want; do
        [ -n "$want" ] || continue
        if grep -qF "DNS:$want" "$WORK/leaf.txt"; then
            echo "  ok   leaf carries $want"
        else
            echo "  FAIL: the leaf does NOT carry $want — the machine will fail to present that name" >&2
            echo "fail" > "$WORK/.failed"
        fi
    done
    [ -f "$WORK/.failed" ] && FAILED=yes
fi

# --- 3. a name the authority cannot vouch for is REFUSED ----------------------
#
# The safety half of the ruling: an explicit list can be explicitly wrong, and
# an out-of-constraint name mints cleanly and fails much later, in a browser,
# saying nothing about where it came from. **Mint time is the only cheap moment
# to catch it.**
echo ""
echo "--- 3. a name outside the constraint is refused"
if "$PROVISION" ca server "$host_label.example.com" "$CA" >"$WORK/refuse.log" 2>&1; then
    fail "an authority constrained to $SUFFIXES minted a leaf for $host_label.example.com"
else
    ok "refused"
    # The refusal has to say what WOULD have worked. A person is reading a list
    # frozen at a mint they performed months ago, in a file they never opened.
    for s in $(echo "$SUFFIXES" | tr ',' ' '); do
        grep -qF "$s" "$WORK/refuse.log" \
            && ok "the refusal names the permitted suffix $s" \
            || fail "the refusal does not say that $s was permitted: $(cat "$WORK/refuse.log")"
    done
fi

# **A bare label is the §156.2 defect itself**, so it is asserted by name — and
# it is asserted against `--names`, because a bare label is exactly what the
# composing form now TAKES. The defect is issuing a certificate whose only name
# is `DNS:arbor`, not typing the word.
if "$PROVISION" ca server --names "$host_label" "$CA" >"$WORK/bare.log" 2>&1; then
    fail "a bare label '$host_label' minted — this is PLAN.md §156.2 returning"
else
    ok "a bare label is refused as a literal name ($host_label is under no suffix)"
fi

echo ""
if [ -n "$FAILED" ]; then
    echo "rehearse.sh: FAIL — the ceremony ran and produced the wrong artifact."
    exit 1
fi
echo "rehearse.sh: PASS — the ceremony ran AND the artifacts are what was asked for."
