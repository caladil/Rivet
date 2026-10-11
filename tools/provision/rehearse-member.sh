#!/bin/sh
# Rehearse a FRESH fleet: a fleet server and then a member of it, each through
# `cargo rivet provision`'s own interview under a pty, in one scratch `--at` —
# and assert what each run left on disk and in the image it prepared.
#
# Usage:
#   tools/provision/rehearse-member.sh          # in the container; needs a pty
#
# ## Why this exists
#
# `rehearse.sh` and `rehearse-menu.sh` rehearse the AUTHORITY and one server
# certificate, and drive `rivet-provision` directly. Neither provisions a
# member, and neither runs the orchestrator a person types. A member's card is
# the one whose trust in its server is easiest to lose without a sound: its
# `peers` must name the key the server's `rivet-fleetd` presents, and a member
# staged without it boots, is listed by its server, and cannot be managed by
# it. This harness walks server-then-member the way a fresh fleet is built and
# asserts that key arrives.
#
# ## What stands in for what, said plainly
#
# * **No card is written.** The interview's device menu is answered `x`; the
#   container sees no removable device in any case. The image is a SYNTHETIC
#   16 MiB GPT with two FAT boot partitions and a `rivet.toml` declaring this
#   tool's schema — real `cargo rivet image` signing, real zstd, real
#   verification and decompression, but no kernel and no `/system`. Its
#   VERSION is the half that is real: it is published once as
#   `PROXY_IN_IMAGE_SINCE` (0.1.114) and once as 0.1.113, which is all the
#   `ADR-060` §5 check reads.
# * **The Armada record is written by this script.** In a real run the
#   server's card is written and verified, the register gains its record with
#   its `rivet-fleetd` operator key, and provisioning adds it to the Armada
#   (`cargo rivet provision`'s own recording step, unit-tested in
#   `provision_tests.rs`). No card is written here, so nothing is recorded, and
#   this script writes the ONE Armada record that step would have written —
#   from the server's own staged register and identity, not from a constant —
#   so the member interview reads it exactly as it would read the real one.
# * **The scratch image an interactive run prepares is removed when it ends**,
#   so the image assertions come from a second, non-interactive pass per role
#   with a literal `--image` file, which is modified in place and kept.
#
# ## HOME is a sentinel
#
# `--at` governs the whole run. `HOME` and `USERPROFILE` point at a directory
# that must never come to exist, and the last check asserts it did not.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

WORK=${RIVET_REHEARSE_DIR:-$(mktemp -d 2>/dev/null || echo "/tmp/rivet-member.$$")}
AT="$WORK/at"
FLEET="$WORK/fleet"
FLEET_OLD="$WORK/fleet-old"
SENTINEL="$WORK/home-MUST-NOT-EXIST"
SERVER=rehearsal-server
MEMBER=rehearsal-member
SERVER_ADDRESS=192.0.2.10
SERVER_GATEWAY=192.0.2.1
# One more than musl asks, which the Nameservers prompt must refuse.
FOUR_NAMESERVERS="192.0.2.53, 192.0.2.54, 192.0.2.55, 192.0.2.56"
FLOOR=0.1.114
BELOW=0.1.113
# boot-a starts at sector 2048: this is its byte offset, for mtools.
BOOT_A=1048576

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

echo "rehearse-member.sh: a fresh $SERVER, then $MEMBER, in $WORK"
echo ""

# --- what this run cannot do without -------------------------------------------
#
# Refusals rather than skips: a harness whose assertions turn themselves off
# passes while proving nothing.
for tool in script sgdisk mkfs.vfat mcopy mdir mtype openssl od; do
    command -v "$tool" >/dev/null 2>&1 \
        || { echo "  FAIL: no $tool in this environment" >&2; exit 1; }
done

# **A removable device is a reason to stop, not to proceed carefully.** The
# interview offers the removable devices; this script answers `x` and never a
# number, and it will not drive that menu at all with a card attached.
for flag in /sys/block/*/removable; do
    [ -r "$flag" ] || continue
    if [ "$(cat "$flag")" = 1 ]; then
        echo "  FAIL: $(dirname "$flag") is removable; this rehearsal drives a device menu and refuses to with one attached" >&2
        exit 1
    fi
done

echo "rehearse-member.sh: building cargo-rivet and rivet-provision from this tree..."
( cd "$REPO" && cargo build --release -p cargo-rivet -p rivetos-provision >/dev/null 2>&1 ) \
    || { echo "  FAIL: cargo-rivet or rivet-provision does not build" >&2; exit 1; }
TARGET=${CARGO_TARGET_DIR:-$REPO/target}
CR="$TARGET/release/cargo-rivet"
[ -x "$CR" ] || { echo "  FAIL: no cargo-rivet binary under $TARGET/release" >&2; exit 1; }
[ -x "$TARGET/release/rivet-provision" ] \
    || { echo "  FAIL: no rivet-provision beside cargo-rivet under $TARGET/release" >&2; exit 1; }

# **Every cargo-rivet run below: the sentinel home, nothing typed, and no
# credential the environment might name.** `tools/container/exec.sh` mounts the
# maintainer's own seeds when his shell names them; a rehearsal must never sign
# with, or even see, a real key, so every variable that could point at one is
# removed rather than trusted to be absent.
SCRUB="-u RIVET_APP_SEED -u RIVET_SYSTEM_SEED -u RIVET_IDENTITY -u RIVET_AT -u RIVET_FLEET_REGISTER -u RIVET_FLEET_REPOSITORY"
run() {
    # shellcheck disable=SC2086
    env $SCRUB HOME="$SENTINEL" USERPROFILE="$SENTINEL" "$CR" "$@" </dev/null
}
# The same, against the fleet repository named first.
run_in() {
    repository=$1
    shift
    # shellcheck disable=SC2086
    env $SCRUB HOME="$SENTINEL" USERPROFILE="$SENTINEL" RIVET_FLEET_REPOSITORY="$repository" \
        "$CR" "$@" </dev/null
}

# --- 0. the workstation: credentials, a repository, an image, fleetd -----------
echo ""
echo "--- 0. credentials, fleet repositories, a published image and rivet-fleetd"
mkdir -p "$WORK/rvt" "$WORK/rvt-old" "$WORK/fleetd"
run setup --at "$AT" --name "Rehearsal Operator" >"$WORK/setup.log" 2>&1 \
    || { cat "$WORK/setup.log" >&2; fail "cargo rivet setup refused"; exit 1; }
run fleet init --at "$FLEET" >"$WORK/init.log" 2>&1 \
    || { cat "$WORK/init.log" >&2; fail "fleet init refused"; exit 1; }
run fleet init --at "$FLEET_OLD" >"$WORK/init-old.log" 2>&1 \
    || { cat "$WORK/init-old.log" >&2; fail "fleet init refused"; exit 1; }

# The synthetic card: two FAT boot partitions labelled as `write-image` looks
# for them, and a `rivet.toml` naming the schema `cargo rivet image` checks.
IMG="$WORK/synthetic-card.img"
truncate -s 16M "$IMG"
sgdisk -o -n 1:2048:14335 -c 1:boot-a -n 2:14336:26623 -c 2:boot-b "$IMG" >/dev/null
# Quiet: mkfs.vfat warns that the FILE is larger than the filesystem asked
# for, which is the point of an offset and not a fault.
mkfs.vfat --offset 2048 "$IMG" 6144 >/dev/null 2>&1
mkfs.vfat --offset 14336 "$IMG" 6144 >/dev/null 2>&1
printf 'version = 1\n' >"$WORK/image-rivet.toml"
MTOOLS_SKIP_CHECK=1 mcopy -i "$IMG@@$BOOT_A" "$WORK/image-rivet.toml" ::rivet.toml

publish_image() {
    run image --img "$IMG" --version "$1" --arch aarch64 --platform rpi4 \
        --signing-key "$AT/keys/rivet-system.seed" --out-dir "$3" --at "$2" \
        >"$WORK/image-$1.log" 2>&1 \
        || { cat "$WORK/image-$1.log" >&2; fail "cargo rivet image $1 refused"; exit 1; }
}
publish_image "$FLOOR" "$FLEET" "$WORK/rvt"
publish_image "$BELOW" "$FLEET_OLD" "$WORK/rvt-old"
ok "a synthetic card published as $FLOOR, and as $BELOW into a second repository"

# A fictional rivet-fleetd, really packaged and signed: a server carries it.
cat_manifest() {
    printf '[workspace]\n\n[package]\nname = "rivet-fleetd"\nversion = "0.1.0"\n'
    printf 'edition = "2024"\npublish = false\n\n[package.metadata.rivet]\n'
    printf 'kind = "daemon"\nrestart = "on-failure"\nready = "listening"\n'
    printf 'network = "group:media"\nstate = "private"\nmemory = "128M"\nlisten = ["tcp:8080"]\n'
}
cat_manifest >"$WORK/fleetd/Cargo.toml"
mkdir -p "$WORK/fleetd/src"
printf 'fn main() {}\n' >"$WORK/fleetd/src/main.rs"
head -c 4096 /dev/zero >"$WORK/fleetd/fake.bin"
run package --manifest-path "$WORK/fleetd/Cargo.toml" --binary "$WORK/fleetd/fake.bin" \
    --signing-key "$AT/keys/rivet-app.seed" --out-dir "$WORK/fleetd" >"$WORK/package.log" 2>&1 \
    || { cat "$WORK/package.log" >&2; fail "cargo rivet package refused"; exit 1; }
FLEETD_RVT=$(ls "$WORK/fleetd"/*.rvt | head -1)
run fleet publish "$FLEETD_RVT" --trusted-key "$AT/keys/rivet-app.pub" --at "$FLEET" \
    >"$WORK/publish.log" 2>&1 \
    || { cat "$WORK/publish.log" >&2; fail "fleet publish refused"; exit 1; }
ok "a fictional rivet-fleetd published"

# --- the interview, under a pty ------------------------------------------------
#
# `script -q -c` is the pty; `rehearse-menu.sh` says why a pipe would prove
# less. `cargo rivet provision` is what runs, and it runs `rivet-provision` as
# its interview — the sibling binary, from a directory that is not a checkout.
interview() {
    keys=$1
    log=$2
    script -q -c "cd '$WORK' && env $SCRUB HOME='$SENTINEL' USERPROFILE='$SENTINEL' \
RIVET_FLEET_REPOSITORY='$FLEET' '$CR' provision --at '$AT'" /dev/null <"$keys" >"$log" 2>&1
}

# --- 1. the server --------------------------------------------------------------
#
# Every line is a keystroke, in order. A choice is TYPED as its word, which the
# menu accepts as readily as its number, so a reordered list cannot move an
# answer onto a different choice. The device menu gets `x` and nothing else,
# and no `y` follows any line that could land on it.
echo ""
echo "--- 1. a fresh fleet server, through the interview"
{
    echo "new"                               # new fleet, or join one? (ADR-063 8)
    echo "rehearsal-fleet"                   # fleet name
    echo "local,rehearsal-fleet.home.arpa"   # suffixes
    echo "y"                                 # keep these? [Y/n]
    echo "Rehearsal Operator"                # operator name
    echo "1"; echo "$SERVER"                 # Hostname
    echo "2"; echo "static"                  # Addressing
    echo "3"; echo "$SERVER_ADDRESS/24"      # Address
    echo "4"; echo "$SERVER_GATEWAY"         # Gateway
    # Nameservers, asked because the address is static: four first, which
    # must be refused, then a bare Return, which takes the gateway.
    echo "10"; echo "$FOUR_NAMESERVERS"      # Nameservers -- refused
    echo "10"; echo ""                       # Nameservers -- the gateway
    echo "5"; echo "kiosk"                   # Mode
    echo "6"; echo "server"                  # Fleet
    echo "7"; echo "America/Chicago"         # Timezone
    echo "8"; echo "rivet-launcher"          # Auto-run
    echo "w"                                 # write
    echo "x"                                 # the device menu: write nothing
} >"$WORK/server.keys"
interview "$WORK/server.keys" "$WORK/server.log" \
    && ok "the server's interview ran to the end" \
    || { cat "$WORK/server.log" >&2; fail "the server's interview exited non-zero"; }
grep -q "othing was recorded" "$WORK/server.log" \
    && ok "nothing was written to a device, and the run says so" \
    || { cat "$WORK/server.log" >&2; fail "the server run did not end at the device menu"; }

SERVER_DOC="$AT/provision/$SERVER.provisioning.toml"
SERVER_BUNDLE="$AT/provision/$SERVER.staged.txt"
SERVER_ID="$AT/identity/$SERVER"
[ -s "$SERVER_DOC" ] && ok "the server's document was written" || fail "no $SERVER_DOC"
[ -s "$SERVER_BUNDLE" ] && ok "the server's staging bundle was written" || fail "no $SERVER_BUNDLE"
[ -s "$SERVER_ID/device.crt" ] && ok "the server's identity was minted" || fail "no identity at $SERVER_ID"

# --- 1a. what the server's bundle holds ------------------------------------------
staged() { grep -q "^$2 $3 " "$1"; }
for f in tls-cert tls-key tls-root; do
    staged "$SERVER_BUNDLE" rivet-proxyd "$f" \
        && ok "server bundle: rivet-proxyd $f" \
        || fail "the server bundle has no rivet-proxyd $f"
done
for f in server.crt server.key ca.crt operator-device.seed.hex operator-device.crt.hex; do
    staged "$SERVER_BUNDLE" rivet-fleetd "$f" \
        && ok "server bundle: rivet-fleetd $f" \
        || fail "the server bundle has no rivet-fleetd $f"
done
records=$(grep -c "^rivet-fleetd register/devices/" "$SERVER_BUNDLE" || true)
[ "$records" = 1 ] && ok "the staged register holds ONE record" \
    || fail "the staged register holds $records records; a fresh server holds itself only"

# A bundle line's contents, as text.
decode() {
    hex=$(grep "^$2 $3 " "$1" | head -1 | cut -d' ' -f3)
    printf '%b' "$(printf '%s' "$hex" | sed 's/../\\x&/g')"
}
decode "$SERVER_BUNDLE" rivet-fleetd "$(grep -o '^rivet-fleetd register/devices/[^ ]*' "$SERVER_BUNDLE" | cut -d' ' -f2)" \
    >"$WORK/server-record.toml"
grep -q "$SERVER" "$WORK/server-record.toml" && grep -q "$SERVER_ADDRESS" "$WORK/server-record.toml" \
    && ok "that record is the server itself, at its static address" \
    || { cat "$WORK/server-record.toml" >&2; fail "the one record is not the server's own"; }

# **fleetd's operator key**, out of the staged register rather than assumed.
FLEETD_KEY=$(decode "$SERVER_BUNDLE" rivet-fleetd register/register.toml | grep -o '[0-9a-f]\{64\}' | head -1)
[ -n "$FLEETD_KEY" ] && ok "the staged register names fleetd's operator key" \
    || fail "the staged register names no operator key"

# The operator's own key, out of the identity `setup` minted.
key_of() {
    openssl x509 -inform DER -in "$1" -noout -pubkey 2>/dev/null \
        | openssl pkey -pubin -outform DER 2>/dev/null | tail -c 32 | od -An -tx1 | tr -d ' \n'
}
OPERATOR_KEY=$(key_of "$AT/identity/operator/device.crt")
SERVER_PIN=$(key_of "$SERVER_ID/device.crt")
[ ${#OPERATOR_KEY} = 64 ] && [ ${#SERVER_PIN} = 64 ] \
    || fail "could not read the operator's key or the server's pin back out of a certificate"

grep -qx "$FLEETD_KEY" "$SERVER_ID/peers" && ok "the server's peers names fleetd's operator key" \
    || fail "the server's peers does not name fleetd's operator key"
grep -qx "$OPERATOR_KEY" "$SERVER_ID/peers" && ok "the server's peers names the operator" \
    || fail "the server's peers does not name the operator"

# --- 1b. what the server's document enables -------------------------------------
enables() { grep -q "^\[services\.$2\]$" "$1"; }
for s in rivet-fleetd rivet-proxyd rivet-displayd rivet-launcher; do
    enables "$SERVER_DOC" "$s" && ok "the server's document enables $s" \
        || fail "the server's document does not enable $s"
done
grep -q '^\[carry\.rivet-fleetd\]$' "$SERVER_DOC" && ok "the server carries rivet-fleetd" \
    || fail "the server's document carries no rivet-fleetd"
carries=$(grep -c '^\[carry\.' "$SERVER_DOC" || true)
[ "$carries" = 1 ] && ok "and carries nothing else" || fail "the server's document carries $carries products"

# --- 1c. a static server is asked its nameservers ---------------------------------
#
# Without `[network.resolver]` a static machine has no resolver at all and
# every lookup fails inside whichever application made it. The prompt offered
# the gateway as its default, refused four by count, and a bare Return took
# the gateway into the document.
grep -qF "Nameservers [$SERVER_GATEWAY] >" "$WORK/server.log" \
    && ok "the Nameservers prompt offered the gateway as its default" \
    || fail "the Nameservers prompt did not appear, or did not offer $SERVER_GATEWAY"
grep -qF "4 given, at most 3" "$WORK/server.log" \
    && ok "four nameservers were refused at the prompt" \
    || fail "four nameservers were not refused by count"
grep -q '^\[network\.resolver\]$' "$SERVER_DOC" \
    && ok "the server's document carries [network.resolver]" \
    || { cat "$SERVER_DOC" >&2; fail "the static server's document has no [network.resolver]"; }
grep -qx "nameservers = \[\"$SERVER_GATEWAY\"\]" "$SERVER_DOC" \
    && ok "and its nameserver is the gateway a bare Return took" \
    || { cat "$SERVER_DOC" >&2; fail "the server's nameservers are not [\"$SERVER_GATEWAY\"]"; }

# --- 2. what a verified write would have recorded -------------------------------
#
# See the header: this is the ONE Armada record the server's recording step
# writes after a verified card, composed from the server's own files.
echo ""
echo "--- 2. the Armada record a written server card would have produced (stand-in)"
mkdir -p "$AT/armada/fleets"
{
    printf 'name = "%s"\n\n[server]\n' "$SERVER"
    printf 'host = "%s:7777"\npin = "%s"\noperator_key = "%s"\n' "$SERVER_ADDRESS" "$SERVER_PIN" "$FLEETD_KEY"
} >"$AT/armada/fleets/$SERVER.toml"
ok "written: $AT/armada/fleets/$SERVER.toml"

# --- 3. the member ---------------------------------------------------------------
#
# The same `--at`: the authority exists now, so the interview asks nothing
# before its menu.
echo ""
echo "--- 3. a fresh member of that server, through the interview"
{
    echo "1"; echo "$MEMBER"                 # Hostname
    echo "5"; echo "kiosk"                   # Mode
    echo "6"; echo "member"                  # Fleet
    echo "7"; echo "America/Chicago"         # Timezone
    echo "8"; echo "rivet-launcher"          # Auto-run
    echo "w"                                 # write
    echo "x"                                 # the device menu: write nothing
} >"$WORK/member.keys"
interview "$WORK/member.keys" "$WORK/member.log" \
    && ok "the member's interview ran to the end" \
    || { cat "$WORK/member.log" >&2; fail "the member's interview exited non-zero"; }
grep -q "othing was recorded" "$WORK/member.log" \
    && ok "nothing was written to a device" \
    || { cat "$WORK/member.log" >&2; fail "the member run did not end at the device menu"; }
grep -q "resume which" "$WORK/member.log" && fail "the member run offered to resume something" || true

MEMBER_DOC="$AT/provision/$MEMBER.provisioning.toml"
MEMBER_BUNDLE="$AT/provision/$MEMBER.staged.txt"
MEMBER_ID="$AT/identity/$MEMBER"
[ -s "$MEMBER_DOC" ] && ok "the member's document was written" || fail "no $MEMBER_DOC"
[ -s "$MEMBER_BUNDLE" ] && ok "the member's staging bundle was written" || fail "no $MEMBER_BUNDLE"

for f in tls-cert tls-key tls-root; do
    staged "$MEMBER_BUNDLE" rivet-proxyd "$f" \
        && ok "member bundle: rivet-proxyd $f" \
        || fail "the member bundle has no rivet-proxyd $f"
done
grep -q "^rivet-fleetd " "$MEMBER_BUNDLE" && fail "the member bundle stages something for rivet-fleetd" \
    || ok "the member bundle stages nothing for rivet-fleetd"

# **The key this whole harness is for.**
grep -qx "$FLEETD_KEY" "$MEMBER_ID/peers" \
    && ok "the member's peers names its server's fleetd operator key" \
    || { cat "$MEMBER_ID/peers" >&2; fail "the member's peers does NOT name its server's fleetd key — the server could list it and manage nothing"; }
grep -qx "$OPERATOR_KEY" "$MEMBER_ID/peers" && ok "the member's peers names the operator" \
    || fail "the member's peers does not name the operator"

for s in rivet-proxyd rivet-displayd rivet-launcher; do
    enables "$MEMBER_DOC" "$s" && ok "the member's document enables $s" \
        || fail "the member's document does not enable $s"
done
enables "$MEMBER_DOC" rivet-fleetd && fail "the member's document enables rivet-fleetd" || true
grep -q '^\[carry\.' "$MEMBER_DOC" && fail "the member's document carries a product" \
    || ok "the member's document carries nothing"

# **A dhcp member is asked no nameservers and declares no resolver**: its
# lease fills the resolver, and a table would override the lease. Not asked
# means not on the screen and no prompt, so the word is absent from the run.
grep -q "Nameservers" "$WORK/member.log" \
    && fail "the dhcp member's interview showed or asked Nameservers" \
    || ok "the dhcp member was not asked for nameservers"
grep -q '^\[network\.resolver\]' "$MEMBER_DOC" \
    && { cat "$MEMBER_DOC" >&2; fail "the dhcp member's document declares a resolver"; } \
    || ok "the member's document declares no resolver, so the lease supplies it"

# --- 4. the prepared images ------------------------------------------------------
#
# The same documents, identities and bundles, written into a literal image
# file that is kept: no terminal, no device, nothing recorded.
echo ""
echo "--- 4. the image each card would be written from"
prepare() {
    cp "$IMG" "$WORK/$1-card.img"
    run_in "$FLEET" provision --document "$2" --identity "$3" \
        --image "$WORK/$1-card.img" --at "$AT" >"$WORK/$1-prepare.log" 2>&1
}
card_files() { MTOOLS_SKIP_CHECK=1 mdir -i "$WORK/$1-card.img@@$BOOT_A" :: 2>/dev/null; }
card_file() { MTOOLS_SKIP_CHECK=1 mtype -i "$WORK/$1-card.img@@$BOOT_A" "::$2" 2>/dev/null; }

prepare server "$SERVER_DOC" "$SERVER_ID" \
    && ok "the server's image was prepared" \
    || { cat "$WORK/server-prepare.log" >&2; fail "preparing the server's image was refused"; }
prepare member "$MEMBER_DOC" "$MEMBER_ID" \
    && ok "the member's image was prepared" \
    || { cat "$WORK/member-prepare.log" >&2; fail "preparing the member's image was refused"; }

card_files server >"$WORK/server-card.ls"
card_files member >"$WORK/member-card.ls"
grep -qi "app-rivet-fleetd" "$WORK/server-card.ls" && ok "the server's card carries app-rivet-fleetd.rvt" \
    || { cat "$WORK/server-card.ls" >&2; fail "the server's card does not carry rivet-fleetd"; }
server_apps=$(grep -ci "app-" "$WORK/server-card.ls" || true)
[ "$server_apps" = 1 ] && ok "and no other application payload" \
    || { cat "$WORK/server-card.ls" >&2; fail "the server's card carries $server_apps payloads"; }
grep -qi "app-rivet-proxyd" "$WORK/server-card.ls" "$WORK/member-card.ls" \
    && fail "a card carries a rivet-proxyd payload" \
    || ok "neither card carries a rivet-proxyd payload"
grep -qi "app-" "$WORK/member-card.ls" \
    && { cat "$WORK/member-card.ls" >&2; fail "the member's card carries an application payload"; } \
    || ok "the member's card carries no application payload at all"
for role in server member; do
    grep -qi "staged" "$WORK/$role-card.ls" && ok "the $role's card carries its staging bundle" \
        || fail "the $role's card carries no staging bundle"
done
card_file server peers | grep -qx "$FLEETD_KEY" \
    && ok "the server's CARD peers names fleetd's operator key" \
    || fail "the server's card peers does not name fleetd's operator key"
card_file member peers | grep -qx "$FLEETD_KEY" \
    && ok "the member's CARD peers names its server's fleetd operator key" \
    || fail "the member's card peers does not name its server's fleetd key"

# --- 5. a member document with no bundle has one built ---------------------------
echo ""
echo "--- 5. a member document handed over with no bundle beside it"
mkdir -p "$WORK/handed-over"
cp "$MEMBER_DOC" "$WORK/handed-over/$MEMBER.provisioning.toml"
prepare built "$WORK/handed-over/$MEMBER.provisioning.toml" "$MEMBER_ID" \
    && ok "it was prepared" \
    || { cat "$WORK/built-prepare.log" >&2; fail "a member document with no bundle was refused"; }
grep -q "rivet-provision stage\` built" "$WORK/built-prepare.log" \
    && ok "the run says it built one" \
    || { cat "$WORK/built-prepare.log" >&2; fail "the run did not say it built a bundle"; }
staged "$WORK/handed-over/$MEMBER.staged.txt" rivet-proxyd tls-cert 2>/dev/null \
    && ok "the built bundle is beside the document, with the front door's certificate" \
    || fail "no built bundle beside the handed-over document"
copies=$(grep -c "^$FLEETD_KEY$" "$MEMBER_ID/peers" || true)
[ "$copies" = 1 ] && ok "building it again left exactly one copy of the server's key in peers" \
    || fail "the member's peers names the server's key $copies times, not once"

# --- 6. an image older than the in-image proxy is refused, for both roles --------
echo ""
echo "--- 6. an image older than $FLOOR is refused (ADR-060 §5)"
OLD_DIGEST=$(sed -n 's/^[[:space:]]*digest[[:space:]]*=[[:space:]]*"\([0-9a-f]*\)".*$/\1/p' \
    "$FLEET_OLD/manifest.toml" | head -1)
[ -n "$OLD_DIGEST" ] || { cat "$FLEET_OLD/manifest.toml" >&2; fail "the $BELOW image has no digest in its manifest"; }
for role in server member; do
    if [ "$role" = server ]; then doc=$SERVER_DOC; id=$SERVER_ID; else doc=$MEMBER_DOC; id=$MEMBER_ID; fi
    if run_in "$FLEET_OLD" provision --document "$doc" --identity "$id" \
        --image "$OLD_DIGEST" --at "$AT" >"$WORK/$role-old.log" 2>&1; then
        cat "$WORK/$role-old.log" >&2
        fail "a $role card was prepared from a $BELOW image"
    elif grep -q "NOTHING WAS WRITTEN" "$WORK/$role-old.log" && grep -q "$BELOW" "$WORK/$role-old.log" \
        && grep -q "$FLOOR" "$WORK/$role-old.log"; then
        ok "a $role card is refused a $BELOW image, naming $FLOOR"
    else
        cat "$WORK/$role-old.log" >&2
        fail "the $role's refusal does not name the image and the floor"
    fi
done

# --- 7. nothing reached a real home ----------------------------------------------
echo ""
[ -e "$SENTINEL" ] && fail "$SENTINEL exists: something read or wrote through HOME instead of --at" \
    || ok "nothing was read or written through HOME"

echo ""
if [ -n "$FAILED" ]; then
    echo "rehearse-member.sh: FAIL — a fresh server and member did not come out as they must."
    exit 1
fi
echo "rehearse-member.sh: PASS — a fresh server and a member of it, through the interview, carry what they must."
