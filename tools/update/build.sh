#!/bin/sh
# Build a signed system-update .rvt from a card image this repository built.
#
# **This is the second committed recipe, and it exists for tools/card/build.sh's
# reason**, stated there and applying here word for word:
#
#   a build that lives in one person's scrollback is a build nobody else can
#   reproduce and nobody can review.
#
# Until 2026-08-25 every update payload in `out/` was produced by hand:
# `sgdisk -p` to read the offsets, three `dd`s, two `mcopy`s, and
# `cargo rivet update` with six flags. `PLAN.md` §0.12 describes the *result*
# and not the steps. This script is the steps.
#
# Usage:
#   ./tools/update/build.sh <card.img> <describe>
#
#   <card.img>   a whole-device image from tools/card/build.sh
#   <describe>   the version string that image was built with. Pass what
#                `git describe --tags --always --dirty` says, asked of WINDOWS
#                git and not WSL's (AGENTS.md §10.5) — `cargo rivet update`
#                refuses to compose one, because it did not build the image and
#                a version here would claim a commit on somebody else's behalf
#                (ADR-027 §3.2.1).
#
# Environment:
#   RIVET_SYSTEM_SEED  the system signing seed. No default: AGENTS.md §8.3
#                        forbids an absolute path in a tracked file, so this
#                        script cannot say where yours is.
#   RIVET_OUT_DIR      where the .rvt lands. Default: out
#   RIVET_KEEP_PAYLOADS  how many update payloads to leave in RIVET_OUT_DIR.
#                        Default: 3 — the store's own rule (ADR-052 §7.2.2).
#                        Older *-update.rvt files are removed after a
#                        successful build, and each is named as it goes.
#                        RIVET_KEEP_PAYLOADS=0 keeps everything. A count of its
#                        own, and NOT RIVET_KEEP, which is the card build's: see
#                        the retention note near the end of this file.
#
# ## Why there is no loop device
#
# `losetup` needs root, and root in WSL needs a password this script must not
# ask for. It is also unnecessary: a partition is a byte range in a file, and
# every tool needed reads one without mounting anything.
#
#   * `sgdisk -p` reads the GPT out of the image file.
#   * `dd` copies the /system partition — it is an ext4 filesystem image, which
#     is exactly what `cargo rivet update --system` wants.
#   * `mcopy -i <fat image>` reads the FAT boot partitions without mounting
#     them, which is the same trick `tools/card/build.sh` uses to write them.
#
# Nothing here is mounted, nothing needs privilege, and no block device is
# named — so the hazard AGENTS.md §10.4 is about cannot arise from this file.
#
# ## The partition numbers are ADR-018 §4's and are not looked up
#
# p1 boot-a, p2 boot-b, p3 system-a, p4 system-b, p5 state. They are asserted
# against the image's own GPT labels below rather than trusted, because an
# image built by a future layout would otherwise be silently mis-extracted.
set -eu

# ---------------------------------------------------------------------------
# Where the time goes -- added 2026-08-30
# ---------------------------------------------------------------------------
#
# `tools/card/build.sh` grew the same thing minutes earlier and it immediately
# found an 80% cost nobody had guessed. This script was the LARGER half of the
# same complaint -- 499 seconds against the card build's 154 -- so it gets the
# same treatment rather than a second round of guessing.
PHASE_LOG=""
PHASE_CURRENT=""
PHASE_BEGAN=$(date +%s)
BUILD_BEGAN=$PHASE_BEGAN

phase() {
    _now=$(date +%s)
    if [ -n "$PHASE_CURRENT" ]; then
        PHASE_LOG="$PHASE_LOG$((_now - PHASE_BEGAN)) $PHASE_CURRENT
"
    fi
    PHASE_CURRENT=${1:-}
    PHASE_BEGAN=$_now
}

report_phases() {
    phase
    echo ""
    echo "--- where the time went ---"
    printf '%s' "$PHASE_LOG" | sort -n | while read -r _secs _name; do
        [ -z "$_name" ] && continue
        printf '  %4ss  %s\n' "$_secs" "$_name"
    done
    printf '  %4ss  TOTAL\n' "$(( $(date +%s) - BUILD_BEGAN ))"
}

usage() {
    sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

[ $# -eq 2 ] || usage
IMAGE=$1
DESCRIBE=$2

[ -f "$IMAGE" ] || { echo "no such image: $IMAGE" >&2; exit 1; }

SEED=${RIVET_SYSTEM_SEED:-}
[ -n "$SEED" ] || {
    echo "RIVET_SYSTEM_SEED is not set, and there is deliberately no default." >&2
    echo "It is the 32-byte system signing seed. See keys/README.md." >&2
    exit 1
}
[ -f "$SEED" ] || { echo "no such signing seed: $SEED" >&2; exit 1; }

OUT_DIR=${RIVET_OUT_DIR:-out}
WORK=$OUT_DIR/update-work

for tool in sgdisk dd mcopy cargo; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "$tool is not on PATH. sgdisk and mcopy live in the archlinux WSL distro" >&2
        echo "(AGENTS.md §10.5); cargo is wherever your rustup put it." >&2
        exit 1
    }
done

echo "--- the image's own partition table ---"
sgdisk -p "$IMAGE"

# Read each partition's start and length in sectors, and check the label is the
# one ADR-018 §4 says that number carries. A layout change then FAILS here
# rather than producing a payload assembled out of the wrong bytes.
field() {
    sgdisk -i "$1" "$IMAGE" | sed -n "s/^$2: *//p" | head -1
}
check() {
    got=$(sgdisk -i "$1" "$IMAGE" | sed -n 's/^Partition name: .\(.*\).$/\1/p' | head -1)
    [ "$got" = "$2" ] || {
        echo "partition $1 is named '$got' and ADR-018 §4 says '$2'." >&2
        echo "The layout changed; this script has not. Refusing rather than guessing." >&2
        exit 1
    }
}

check 1 boot-a
check 2 boot-b
check 3 system-a

BOOT_A_START=$(field 1 'First sector' | cut -d' ' -f1)
BOOT_A_LEN=$(field 1 'Partition size' | cut -d' ' -f1)
BOOT_B_START=$(field 2 'First sector' | cut -d' ' -f1)
BOOT_B_LEN=$(field 2 'Partition size' | cut -d' ' -f1)
SYS_START=$(field 3 'First sector' | cut -d' ' -f1)
SYS_LEN=$(field 3 'Partition size' | cut -d' ' -f1)

rm -rf "$WORK"
mkdir -p "$WORK/boot-a" "$WORK/boot-b"

# `--system` takes the filesystem for the slot an update writes. Both slots of a
# freshly built card are identical, so slot A's is as good as slot B's; the
# installer decides which slot it lands in on the board (ADR-018 §4).
echo "--- extracting /system (${SYS_LEN} sectors) ---"
# **4 MiB blocks, offsets in BYTES.** These three were `bs=512` until
# 2026-08-30 -- two million individual reads to copy a gigabyte, three times
# over. `tools/card/build.sh` had the identical bug in its verification step
# and it was 80% of that build; this script does it for the system partition
# and both boot partitions, and was 499 seconds.
#
# `skip_bytes`/`count_bytes` keep each extraction sector-exact while letting
# the block size be whatever is fast, rather than requiring every partition to
# start on a 4 MiB boundary -- which is true of this layout and is not a thing
# to depend on.
phase "extracting the system partition"
dd if="$IMAGE" of="$WORK/system.img" bs=4M iflag=skip_bytes,count_bytes \
    skip=$((SYS_START * 512)) count=$((SYS_LEN * 512)) status=none

echo "--- extracting both boot partitions ---"
phase "extracting the boot partitions"
dd if="$IMAGE" of="$WORK/boot-a.fat" bs=4M iflag=skip_bytes,count_bytes \
    skip=$((BOOT_A_START * 512)) count=$((BOOT_A_LEN * 512)) status=none
dd if="$IMAGE" of="$WORK/boot-b.fat" bs=4M iflag=skip_bytes,count_bytes \
    skip=$((BOOT_B_START * 512)) count=$((BOOT_B_LEN * 512)) status=none
mcopy -i "$WORK/boot-a.fat" -s "::/*" "$WORK/boot-a/"
mcopy -i "$WORK/boot-b.fat" -s "::/*" "$WORK/boot-b/"

# Both directories are read and DIFFED by the tool: a name whose bytes agree
# becomes one invariant member and a name whose bytes differ becomes two
# (ADR-029 §3). Nothing here knows cmdline.txt is the differing one, and the
# tool WARNS if it ever finds a second — see PLAN.md §0.12.
echo "--- signing ---"
phase "packaging and signing"
cargo run -q -p cargo-rivet -- rivet update \
    --system "$WORK/system.img" \
    --boot-a "$WORK/boot-a" \
    --boot-b "$WORK/boot-b" \
    --signing-key "$SEED" \
    --version "$DESCRIBE" \
    --out-dir "$OUT_DIR"

rm -rf "$WORK"

# --- retention: never leave more than RIVET_KEEP_PAYLOADS payloads ----------
#
# `PLAN-Feature-Set-2026-09-12` §3.6. `R-135` removed eighty-six payloads from
# `out/` by hand and recorded that nothing pruned it, so it would happen again;
# `tools/card/build.sh` gained retention for card images, and this script did
# not. On 2026-09-12 `out/` held seventeen update payloads, 12 GB, most of them
# builds nobody would deploy again.
#
# **Three, and a count separate from the card build's `RIVET_KEEP` (two).** Three
# is the machine's own rule for its artifact store, so the workstation keeps what
# a board could hold. It is more than the card count deliberately: the card build
# warns that a payload cannot be rebuilt once its card image is gone, so a
# payload outlives its image as the only copy that does not need a rebuild. One
# variable for both would make keeping more of one mean keeping more of the
# other.
#
# **Newest-first by mtime**, as the card build orders its images, and only
# `*-update.rvt` in the output directory: an application payload, a card image
# or anything else in `out/` is not this script's to remove. The payload this run
# just wrote is the newest file, so it is never among those removed. Runs only
# after a successful build, because `set -eu` has stopped the script before here
# on any failure.
KEEP_PAYLOADS=${RIVET_KEEP_PAYLOADS:-3}
case "$KEEP_PAYLOADS" in
    ''|*[!0-9]*)
        echo "RIVET_KEEP_PAYLOADS=$KEEP_PAYLOADS is not a whole number; nothing was removed." >&2
        KEEP_PAYLOADS=0 ;;
esac

if [ "$KEEP_PAYLOADS" -gt 0 ]; then
    # shellcheck disable=SC2012
    STALE=$(ls -t "$OUT_DIR"/*-update.rvt 2>/dev/null | tail -n +$((KEEP_PAYLOADS + 1)))
    if [ -n "$STALE" ]; then
        echo
        echo "--- retention: keeping the $KEEP_PAYLOADS newest update payloads, removing $(echo "$STALE" | wc -l) ---"
        echo "$STALE" | while IFS= read -r old; do
            [ -n "$old" ] || continue
            printf '    %s (%s)\n' "$(basename "$old")" "$(du -h "$old" | cut -f1)"
            rm -f "$old"
        done
        echo
        echo "    Set RIVET_KEEP_PAYLOADS to change this, or RIVET_KEEP_PAYLOADS=0 to keep every payload."
        echo "    A board's own store keeps three per product as well (ADR-052 §7.2.2)."
    fi
fi

report_phases

cat <<'EOF'

--- what happens next, and none of it happened here ---

Nothing has been sent and nothing has been installed. To put it on a board:

  cargo rivet install --host <host>:7777 --identity <dir> --pin <hex> \
      --rvt <the .rvt above> confirm

which transfers it into that machine's artifact store, checks the board's BLAKE3
against this machine's, runs `rivet apply install <digest> confirm` there — the
digest it just sent (ADR-052 §7.2) — and waits for `rivet get install` to show
that install's NEW correlation finished, exiting non-zero if it failed. That
does NOT reboot: a system update lands in the inactive slot and the installer
arms the next boot as a one-shot trial of it. Taking the trial is one more
decision, which is when the machine goes down (ADR-012 §7.1) —

  rivet apply trial-restart confirm

PID 1 commits a healthy trial itself; `rivet get slot` says `committed: true`
when it has.
EOF
