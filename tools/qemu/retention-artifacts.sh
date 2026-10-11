#!/bin/sh
# tools/qemu/retention-artifacts.sh — **four signed artifacts of one product,
# so retention can be watched on a REAL CARD.**
#
# Run it in the container, which is where the cross toolchain lives:
#
#   RIVET_APP_SEED=$HOME/.rivet/keys/rivet-app.seed \
#       tools/container/exec.sh tools/qemu/retention-artifacts.sh
#
# The files land in `out/retention-proof/` and the script prints, for each one,
# the version, the file name, the BLAKE3 digest and the byte size. Nothing is
# sent anywhere: this script ends at four files on a disk.
#
# ## Why it exists beside `adr-052-proof.sh` rather than inside it
#
# `adr-052-proof.sh` asks `ADR-052` §7.2's four questions of a booted QEMU
# guest, and its third is retention — four versions arrive, one is evicted, the
# object list is read back. What that run cannot say is what happens on a card,
# because `run.sh --card` and `--secrets` are mutually exclusive (that script's
# header argues it at length): a guest with a real ext4 cannot receive a
# transfer at all, so the eviction it watches happens on the initramfs tmpfs.
#
# **The medium is the part left open**, and closing it needs a board, an
# operator and an SD card — which is the maintainer's half, not a script's. So
# this script does the half that IS automatable and stops: it produces the
# artifacts, at versions chosen not to collide with anything already built, and
# prints the digests the store will be keyed by.
#
# ## What a pass looks like, on the machine
#
# `rivet_core::artifact::KEEP` is 3: **the three newest versions of each product
# are kept and older ones evicted.** Deploy these four in ascending order and
# the store should finish holding `0.3.2`, `0.3.3` and `0.3.4`, with `0.3.1`
# evicted by the arrival of `0.3.4` and any earlier `hello-rivet` gone before
# that. The digests printed below are what to look for in `artifacts/objects`.
#
# ## What this script does NOT settle
#
# **It proves nothing about retention.** It produces files. Every claim about
# eviction above is a statement of what the code is supposed to do, to be
# checked against a board by the person driving it — not something four `.rvt`
# files on a host can demonstrate.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# `CARGO_TARGET_DIR` and not `$REPO/target`: the container mounts a named volume
# there. `adr-052-proof.sh` says the same, for the same reason.
TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
HOST_OUT=$TARGET_DIR/release

# **The staged crates live OUTSIDE the workspace**, so none of them inherits
# `.cargo/config.toml`'s cross linker — `adr-052-proof.sh`'s argument, and the
# same one-line answer. `rust-lld` ships with the toolchain (`ADR-026` §3.1).
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER

fail() { echo; echo "retention-artifacts: FAIL: $*" >&2; exit 1; }
note() { echo "retention-artifacts: $*"; }

# **The versions, and why these.** Four that sort unambiguously, in a series no
# build in this repository has used: `hello-rivet`'s own `Cargo.toml` is 0.1.0
# and `adr-052-proof.sh` packages 0.1.1 through 0.1.4, so a 0.3.x series cannot
# be confused with either in a store that has seen both.
#
# **The first argument overrides the series**, so a re-run on a board whose store
# already holds one gets four digests it has never seen: passing `0.4` packages
# 0.4.1 through 0.4.4. An argument rather than an environment variable because
# `tools/container/exec.sh` forwards only the variables it names, and a variable
# it does not name vanishes silently — the first attempt at this packaged 0.3.x
# again and said nothing. A re-run with the same series re-pushes stored
# digests, which tests the transfer's short-circuit rather than retention.
SERIES=${1:-0.3}
V1=$SERIES.1
V2=$SERIES.2
V3=$SERIES.3
V4=$SERIES.4
VERSIONS="$V1 $V2 $V3 $V4"

SRC=$REPO/examples/probes/hello-rivet
[ -d "$SRC" ] || fail "no crate at $SRC"

OUT_DIR=$REPO/out/retention-proof
WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-retention.XXXXXX") \
    || fail "could not create a working directory"
chmod 700 "$WORK"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# --- the tools --------------------------------------------------------------
#
# **Built here, not taken from `PATH`.** An installed `cargo rivet` is whatever
# somebody last ran `cargo install` on; this one is the tree's.
note "building cargo-rivet for the host: the packager"
cargo build --release -p cargo-rivet || fail "cargo build -p cargo-rivet failed"
CARGO_RIVET=$HOST_OUT/cargo-rivet
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"

# **And the CLI, for the HOST, as a second opinion about the digest.**
#
# `cargo rivet package` prints the BLAKE3 it computed and this script believes
# it — `ADR-011`, and it has no hasher of its own. But the digest is the whole
# deliverable here, so it is read back a second time from the finished file by
# a different program: `rivet measure hash` is `cmd/rivet`'s, not the
# packager's. Two agreeing answers also say the copy into `out/` is intact,
# which is the file the maintainer will actually send.
note "building rivetos-cli for the host: the second reader of the digest"
cargo build --release -p rivetos-cli || fail "cargo build -p rivetos-cli failed"
RIVET_CLI=$HOST_OUT/rivet
[ -x "$RIVET_CLI" ] || fail "no rivet at $RIVET_CLI"

# --- the helpers, which are `adr-052-proof.sh`'s ----------------------------
#
# Same method, deliberately: a second way of staging a crate or reading a digest
# would be a second thing to keep true.

# A crate copied out of the tree, at a version this script chooses.
#
# Out of the tree so `rivet_remote::name::Version::compose` finds no work tree
# and the version is the bare one from `Cargo.toml` — no `-g<short>`, no
# `-dirty`. Four artifacts then differ by exactly the field being varied, which
# is what a retention rule that groups by product should be given.
stage_crate() {
    _src=$1
    _dest=$2
    _version=$3
    mkdir -p "$_dest"
    cp -a "$_src/Cargo.toml" "$_src/src" "$_dest/" \
        || fail "could not stage $_src at $_dest"
    # The version, and only the version. `[package]`'s is the first `version =`
    # in the file.
    sed -i "0,/^version = /s/^version = .*/version = \"$_version\"/" "$_dest/Cargo.toml" \
        || fail "could not set the version in $_dest/Cargo.toml"
    grep -q "^version = \"$_version\"\$" "$_dest/Cargo.toml" \
        || fail "the version in $_dest/Cargo.toml is not $_version — the substitution missed"
}

package() {
    _crate=$1
    _report=$2
    "$CARGO_RIVET" package \
        --manifest-path "$_crate/Cargo.toml" \
        --target "$TARGET" \
        --target-dir "$_crate/target" \
        --out-dir "$_crate/rvt" \
        --no-assets \
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

# The finished file's digest, read by `cmd/rivet` rather than by the packager.
measured_of() {
    _f=$1
    _m=$("$RIVET_CLI" measure hash "$_f" 2>&1) \
        || { printf '%s\n' "$_m" >&2; fail "rivet measure hash failed for $_f"; }
    _d=$(printf '%s\n' "$_m" | grep -oE '[0-9a-f]{64}' | head -1)
    [ -n "$_d" ] || { printf '%s\n' "$_m" >&2; fail "rivet measure hash printed no digest for $_f"; }
    printf '%s\n' "$_d"
}

# --- the artifacts ----------------------------------------------------------

note "packaging hello-rivet at $VERSIONS for $TARGET"
for _v in $VERSIONS; do
    stage_crate "$SRC" "$WORK/hello-$_v" "$_v"
    package "$WORK/hello-$_v" "$WORK/package-$_v.log"
done

mkdir -p "$OUT_DIR" || fail "could not create $OUT_DIR"
# **Only this script's own output is cleared**, by the exact names it is about
# to write. A wildcard wider than that would be a script that tidies a directory
# it does not own.
for _v in $VERSIONS; do
    rm -f "$OUT_DIR/hello-rivet-$_v-aarch64-application.rvt"
done

SUMMARY=$WORK/summary
: >"$SUMMARY"

for _v in $VERSIONS; do
    _src_rvt=$(rvt_of "$WORK/hello-$_v")
    _packed=$(blake3_of "$WORK/package-$_v.log")
    _name=$(basename "$_src_rvt")
    cp "$_src_rvt" "$OUT_DIR/$_name" || fail "could not copy $_src_rvt into $OUT_DIR"
    _dest=$OUT_DIR/$_name
    _measured=$(measured_of "$_dest")
    [ "$_packed" = "$_measured" ] \
        || fail "the packager said $_packed for $_name and rivet measure says $_measured.
    One of the two read different bytes — the copy into $OUT_DIR is the suspect."
    _bytes=$(wc -c <"$_dest" | tr -d ' ')
    printf '%s\t%s\t%s\t%s\n' "$_v" "$_name" "$_packed" "$_bytes" >>"$SUMMARY"
done

# **Four distinct digests, or the retention story below is vacuous.** Two
# versions that hashed the same would be one object in the store, and an
# eviction count taken over them would mean nothing.
_count=$(cut -f3 "$SUMMARY" | sort -u | wc -l | tr -d ' ')
[ "$_count" -eq 4 ] \
    || fail "four artifacts produced $_count distinct digests. The store is keyed by digest; duplicates would collapse into one object."

# **Every one verified against the public half of the signing key.** The seed
# was passed to the packager and never read here; this checks what came out of
# it, with the key `tools/card/build.sh` registers — so what verifies here is
# what a card built from a clean checkout trusts.
note "verifying each artifact against keys/rivet-app.pub"
while IFS="$(printf '\t')" read -r _v _name _dig _bytes; do
    "$CARGO_RIVET" inspect "$OUT_DIR/$_name" --trusted-key "$REPO/keys/rivet-app.pub" \
        >"$WORK/inspect-$_v" 2>&1 \
        || { cat "$WORK/inspect-$_v" >&2; fail "$_name does not verify against keys/rivet-app.pub"; }
    grep -q '^verified ' "$WORK/inspect-$_v" \
        || { cat "$WORK/inspect-$_v" >&2; fail "cargo rivet inspect did not report $_name as verified"; }
done <"$SUMMARY"

# --- what to look for -------------------------------------------------------

DIGESTS=$OUT_DIR/digests.txt
{
    echo "hello-rivet — four signed aarch64 artifacts for the retention proof"
    echo "produced by tools/qemu/retention-artifacts.sh"
    echo
    printf '%-8s  %-64s  %10s  %s\n' version blake3 bytes file
    while IFS="$(printf '\t')" read -r _v _name _dig _bytes; do
        printf '%-8s  %-64s  %10s  %s\n' "$_v" "$_dig" "$_bytes" "$_name"
    done <"$SUMMARY"
} >"$DIGESTS"

echo
echo "--- four signed artifacts, in $OUT_DIR ---"
echo
printf '  %-8s  %-64s  %10s  %s\n' version blake3 bytes file
while IFS="$(printf '\t')" read -r _v _name _dig _bytes; do
    printf '  %-8s  %-64s  %10s  %s\n' "$_v" "$_dig" "$_bytes" "$_name"
done <"$SUMMARY"
echo
echo "  Each verified against keys/rivet-app.pub, and each digest read twice —"
echo "  once from cargo rivet package, once from rivet measure hash on the copy"
echo "  above. The list is also at $DIGESTS."
echo
echo "  RETENTION, WHICH THIS SCRIPT DOES NOT TEST: KEEP is 3, so deploying"
echo "  these four in ascending order should leave $V2, $V3 and $V4 in the"
echo "  store and evict $V1 along with any older hello-rivet. That is what the"
echo "  code is supposed to do; only a board can say whether it does."
