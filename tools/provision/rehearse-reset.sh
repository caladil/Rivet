#!/bin/sh
# Rehearse `rivet-reset` — `SPEC-05` §2, step one of the twelve — and assert
# what it removed, what it refused, and which of two identically-named
# authorities it recognised.
#
# Usage:
#   tools/provision/rehearse-reset.sh
#
# ## Nothing here opens a certificate store, and that is the point
#
# `rivet-reset`'s store half deletes certificates, and there is no way to
# rehearse a deletion against the maintainer's real store that is worth what it
# risks — that store took an afternoon to repair by hand on 2026-09-07 and it
# holds certificates belonging to other software. So this script exercises the
# two halves that CAN be proven from files:
#
#   1. **The matcher**, which is the part that has to be right. Two throwaway
#      authorities are minted with the SAME subject name — the 2026-09-07 state
#      exactly — and every certificate of both is put to `--verdict`. Every
#      subject and every issuer in the six is the same string; the answers are
#      not. That is the whole claim, and it needs no store at all.
#   2. **The filesystem**, against a scratch `~/.rivet` built here: what went,
#      what stayed, the seed refusal, and idempotency.
#
# **What is NOT covered, and it is named rather than skipped:** removal from a
# Windows certificate store. `cmd/rivet-reset/src/store.rs` is read-only-tested
# by its own unit test, which enumerates the three stores and asserts they are
# readable. The `Remove-Item -DeleteKey` path is unproven, and the report at the
# end of this script says so rather than letting a pass imply otherwise.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

# **A throwaway tree, and HOME is pointed into it.** `rivet-reset` resolves
# `~/.rivet` from HOME before USERPROFILE, exactly as `rivet-provision` does, so
# setting one variable moves the seeds, the identity directory and the default
# authority together. Every path is ALSO passed explicitly below, because the
# register does not come from HOME — it comes through Ignition's loader — and a
# rehearsal that reached the real register would be the failure it is testing for.
WORK=${RIVET_REHEARSE_DIR:-$(mktemp -d 2>/dev/null || echo "/tmp/rivet-reset-rehearse.$$")}
HOME_=$WORK/home
RIVET=$HOME_/.rivet
CA=$RIVET/ca
KEYS=$RIVET/keys
PROVISION=$RIVET/provision
REGISTER=$WORK/register

FAILED=""
fail() {
    echo "  FAIL: $1" >&2
    FAILED="yes"
}
ok() { echo "  ok   $1"; }

cleanup() {
    if [ -z "$FAILED" ] && [ -z "${RIVET_REHEARSE_KEEP:-}" ]; then
        rm -rf "$WORK"
    else
        echo ""
        echo "  the artifacts are in $WORK"
    fi
}
trap cleanup EXIT

echo "rehearse-reset.sh: working in $WORK"
echo ""

# --- the tools ----------------------------------------------------------------
#
# Built from THIS tree and called by path, as every other harness here does: a
# `rivet-*` binary on a PATH is an older build of this repository, and an older
# build refuses input this one accepts.
echo "rehearse-reset.sh: building rivet-reset and rivet-provision from this tree..."
( cd "$REPO" && cargo build --release -p rivetos-reset -p rivetos-provision >/dev/null 2>&1 ) \
    || { echo "  FAIL: the tools do not build" >&2; exit 1; }
TARGET=${CARGO_TARGET_DIR:-$REPO/target}
RESET="$TARGET/release/rivet-reset"
[ -x "$RESET" ] || RESET="$TARGET/release/rivet-reset.exe"
[ -x "$RESET" ] || { echo "  FAIL: no rivet-reset binary under $TARGET/release" >&2; exit 1; }
MINT="$TARGET/release/rivet-provision"
[ -x "$MINT" ] || MINT="$TARGET/release/rivet-provision.exe"
[ -x "$MINT" ] || { echo "  FAIL: no rivet-provision binary under $TARGET/release" >&2; exit 1; }

# `--no-install` on every mint in this file. Without it the `ca` verb files the
# operator's identity into the real Windows Personal store with `certutil`,
# which Windows resolves from the system directory — so a rehearsal cannot be
# made harmless by adjusting PATH (findings §3.4).
mint() {
    mkdir -p "$2"
    "$MINT" ca "local,home-fleet.home.arpa" "$1" "$2" "Your Name" --no-install \
        </dev/null >"$WORK/mint.log" 2>&1 \
        || { cat "$WORK/mint.log" >&2; return 1; }
    "$MINT" ca server arbor "$2" </dev/null >>"$WORK/mint.log" 2>&1 \
        || { cat "$WORK/mint.log" >&2; return 1; }
}

# One certificate, one answer: `GOING` or `stays`.
verdict() {
    if "$RESET" --root "$1" --verdict "$2" 2>&1 | grep -q "GOING"; then
        echo "GOING"
    else
        echo "stays"
    fi
}

# ==============================================================================
# 1. TWO AUTHORITIES, ONE NAME — the assertion this program exists for
# ==============================================================================
#
# On 2026-09-07 a new authority was minted carrying the subject of the one it
# replaced, both `CN=home-fleet`. Browsers match the issuer NAME, so every
# certificate either authority had ever issued became eligible at once: four
# were offered and none could authenticate (findings §3.1).
#
# **A reset that matched on the name would be wrong in both directions** — it
# would delete the authority being kept, and it could never tell an operator
# which of two identical-looking roots it was about to take. So the matcher
# compares the KEY, and this is the section that proves it against certificates
# `rivet-provision` actually minted rather than against a test fixture.
echo ""
echo "--- 1. two authorities with one name are told apart"
mint home-fleet "$WORK/a" || { fail "authority A would not mint"; exit 1; }
mint home-fleet "$WORK/b" || { fail "authority B would not mint"; exit 1; }

# The names really are identical, and that is asserted rather than assumed: if a
# future `ca` disambiguated them, everything below would pass for the wrong
# reason.
if cmp -s "$WORK/a/root.crt" "$WORK/b/root.crt"; then
    fail "the two authorities are byte-identical — this proves nothing"
else
    ok "two distinct authorities, both minted as CN=home-fleet"
fi

for f in root.crt browser.crt server-arbor.local.crt; do
    got=$(verdict "$WORK/a/root.crt" "$WORK/a/$f")
    [ "$got" = "GOING" ] \
        && ok "A's $f is recognised as A's" \
        || fail "A's own $f was not recognised: $got"
done

# **The half that matters.** Same subject, same issuer, same file names — and
# none of it is A's. A matcher comparing names deletes all three.
for f in root.crt browser.crt server-arbor.local.crt; do
    got=$(verdict "$WORK/a/root.crt" "$WORK/b/$f")
    [ "$got" = "stays" ] \
        && ok "B's $f is LEFT, despite an identical subject and issuer" \
        || fail "B's $f would have been removed by a reset of A — this is findings §3.1"
done

# Symmetric, so neither authority is the privileged one by accident.
got=$(verdict "$WORK/b/root.crt" "$WORK/b/browser.crt")
[ "$got" = "GOING" ] && ok "the same test from B's side agrees" \
    || fail "B does not recognise its own leaf"
got=$(verdict "$WORK/b/root.crt" "$WORK/a/browser.crt")
[ "$got" = "stays" ] && ok "and A's leaf is left when B is the authority" \
    || fail "A's leaf would be removed by a reset of B"

# ==============================================================================
# 2. THE FILESYSTEM HALF
# ==============================================================================
echo ""
echo "--- 2. the scratch workstation"
mkdir -p "$KEYS" "$PROVISION" "$REGISTER/devices"
mint rehearsal-fleet "$CA" || { fail "the scratch authority would not mint"; exit 1; }

# The seeds. **These are the files a reset must refuse**, and they are put here
# so that their survival is measured rather than assumed.
printf 'not a real seed\n' > "$KEYS/rivet-system.seed"
printf 'not a real seed\n' > "$KEYS/rivet-app.seed"

# A register with a device record in it — the August leftover `SPEC-05` §2 opens
# by describing.
printf 'name = "rehearsal"\n' > "$REGISTER/register.toml"
printf 'id = "1f0532852afc2842"\n' > "$REGISTER/devices/1f0532852afc2842.toml"

# Both kinds of file that share `~/.rivet/provision`: the `w` key's composed
# document and the `s` key's answers notebook.
printf 'version = 1\nhostname = "arbor"\n' > "$PROVISION/arbor.provisioning.toml"
printf '# rivet-provision answers\n' > "$PROVISION/arbor.toml"

run_reset() {
    HOME="$HOME_" "$RESET" --ca "$CA" --register "$REGISTER" --provision "$PROVISION" "$@"
}

echo ""
echo "--- 3. the report names everything, and --dry-run removes nothing"
run_reset --dry-run >"$WORK/dry.log" 2>&1 || fail "--dry-run exited non-zero"
for want in "root.key" "register.toml" "arbor.provisioning.toml" "a device record" \
            "an answers notebook" "nothing was removed"; do
    grep -qi -- "$want" "$WORK/dry.log" \
        && ok "the report names $want" \
        || fail "the report does not name $want: see $WORK/dry.log"
done
[ -f "$CA/root.key" ] && ok "--dry-run removed nothing" || fail "--dry-run REMOVED something"

# **The refusal is in the report even on a run that removes nothing**, because
# the sentence is what stops somebody looking for a flag.
grep -q "rivet-app.seed" "$WORK/dry.log" \
    && ok "the seeds are named as left alone" \
    || fail "the report does not name the signing seeds"

echo ""
echo "--- 4. a signing seed in the way REFUSES THE WHOLE RUN (SPEC-05 §2.2)"
# A seed where a reset would reach it. It is not where one belongs — that is the
# point: the guard is on the FILE, so it holds wherever the file turns up, and a
# reset that quietly skipped it and removed the rest would leave a workstation in
# a state nobody described.
cp "$KEYS/rivet-app.seed" "$CA/rivet-app.seed"
if run_reset --confirm >"$WORK/refuse.log" 2>&1; then
    fail "a run with a signing seed in its plan SUCCEEDED"
else
    ok "refused, and non-zero"
fi
grep -q "SIGNING SEED" "$WORK/refuse.log" \
    && ok "the refusal says what it is refusing" \
    || fail "the refusal does not name the seed: $(cat "$WORK/refuse.log")"
grep -q "rivet-app.seed" "$WORK/refuse.log" \
    && ok "and names the file" \
    || fail "the refusal does not name the file"
[ -f "$CA/rivet-app.seed" ] && ok "the seed survived" || fail "THE SEED WAS REMOVED"
# **And nothing else went either.** A refusal that had already removed half the
# authority would be worse than no refusal.
[ -f "$CA/root.key" ] && ok "and so did everything else — nothing was removed" \
    || fail "the refused run removed files anyway"
rm -f "$CA/rivet-app.seed"

echo ""
echo "--- 5. the reset itself"
run_reset --confirm >"$WORK/reset.log" 2>&1 || fail "the reset exited non-zero"
[ -d "$CA" ] && fail "the authority directory is still there" || ok "the authority is gone"
[ -d "$REGISTER" ] && fail "the register is still there" || ok "the register is gone"
[ -d "$PROVISION" ] && fail "the provisioning documents are still there" \
    || ok "the composed documents and the notebook are gone"

# **The three things that must survive it**, checked individually so a failure
# says which.
[ -f "$KEYS/rivet-system.seed" ] && ok "rivet-system.seed survived" \
    || fail "THE SYSTEM SIGNING SEED WAS REMOVED"
[ -f "$KEYS/rivet-app.seed" ] && ok "rivet-app.seed survived" \
    || fail "THE APPLICATION SIGNING SEED WAS REMOVED"
[ -d "$KEYS" ] && ok "~/.rivet/keys survived" || fail "~/.rivet/keys was removed"

echo ""
echo "--- 6. it is idempotent (SPEC-05 §2.3)"
run_reset --confirm >"$WORK/again.log" 2>&1 \
    && ok "a second run is a report, not an error" \
    || fail "a second run failed: $(cat "$WORK/again.log")"
grep -q "already in the state a first-ever run would find" "$WORK/again.log" \
    && ok "and says so" \
    || fail "the second run does not say the workstation is clean"

# ==============================================================================
echo ""
if [ -n "$FAILED" ]; then
    echo "rehearse-reset.sh: FAIL"
    exit 1
fi
echo "rehearse-reset.sh: PASS — the files, the refusal, the idempotency, and the"
echo "  matcher against two authorities that share a name."
echo ""
echo "  UNVERIFIED, and not covered by any of the above: REMOVAL from a Windows"
echo "  certificate store. Reading the three stores is exercised by"
echo "  cmd/rivet-reset's own unit test; Remove-Item -DeleteKey is unproven."
echo "  See SPEC-05 §2.5 for what closes it and why an agent must not."
