#!/bin/sh
# Put a desired-state document on a card image's /state, so that a
# `run.sh --card` boot has a configuration.
#
# Usage:
#   ./seed-card.sh <card.img> --document <file>
#
# **This modifies <card.img> IN PLACE.** Copy it first if the original matters:
#
#   cp out/rivet-…-card.img /tmp/work.img && ./seed-card.sh /tmp/work.img …
#
# ## Why this exists
#
# `run.sh --card` gave the emulator a real `/state` on real ext4 (`PLAN.md`
# §62) and took something away in the same move: **`--document` and `--state`
# stopped working**, because both write into the initramfs under `/state` and
# PID 1 now mounts the card over that directory. `run.sh` refuses both
# combinations by name rather than ignoring them (`PLAN.md` §62.8), which is
# correct and leaves a machine with a card unable to be configured at all.
#
# A board does not have this problem: PID 1 reads the provisioning file off the
# FAT boot partition and seeds `/state/config/machine.toml` from it (`ADR-017`
# §3.1.9). **That path cannot run under `-M virt`** — it needs
# `/proc/device-tree/chosen/bootloader/partition` to know which boot partition
# booted, and the emulator has no such property, so a card built with
# `RIVET_DOCUMENT` does not self-provision here either.
#
# So this writes the document where a machine that has ALREADY provisioned
# keeps it. What it produces is not a first boot — it is **boot 2**, which is
# the state `run.sh --state` existed to imitate and could only imitate on a
# tmpfs. `PLAN.md` §56.4 item 1 — *is the accounts projection idempotent across
# boots* — is the question this makes askable, and it has been unanswerable
# since 2026-08-27 because `/state` never survived one.
#
# ## Why there is no loop device
#
# The same reason `tools/update/build.sh` gives, and it applies word for word:
# `losetup` needs root, root in WSL needs a password this script must not ask
# for, and it is unnecessary. A partition is a byte range in a file. `sgdisk -p`
# reads the GPT out of the file, `dd` copies the range, and `debugfs` reads and
# writes an ext4 image without mounting it. Nothing is mounted, nothing needs
# privilege, and no block device is named — so the hazard `AGENTS.md` §10.4 is
# about cannot arise from this file.
#
# `sgdisk` and `debugfs` live only in the WSL `archlinux` distro. This script
# has to run there.
set -eu

usage() {
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

REPO=$(cd "$(dirname "$0")/../.." && pwd)

[ $# -ge 1 ] || usage
IMAGE=$1
shift

DOCUMENT=
while [ $# -gt 0 ]; do
    case $1 in
        --document)
            [ $# -ge 2 ] || { echo "$0: --document needs a path" >&2; exit 2; }
            DOCUMENT=$2
            shift 2 ;;
        -h|--help) usage ;;
        *)  echo "$0: unknown option '$1'" >&2; exit 2 ;;
    esac
done

if [ -z "$DOCUMENT" ]; then
    echo "$0: nothing to do — pass --document <file>." >&2
    echo "    Seeding a whole /state tree is deliberately NOT supported here:" >&2
    echo "    it means carrying modes and ownership through debugfs for an" >&2
    echo "    arbitrary directory, and the thing every caller actually wants is" >&2
    echo "    a machine with a configuration. Boot the card twice for the rest." >&2
    exit 2
fi

# --- the tools, named rather than assumed -----------------------------------
for tool in sgdisk dd debugfs; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$0: $tool is not on PATH." >&2
        echo "    sgdisk and debugfs live in the archlinux WSL distro, which is" >&2
        echo "    where this script has to run (AGENTS.md §10.5)." >&2
        exit 1
    fi
done

# --- the inputs -------------------------------------------------------------
[ -f "$IMAGE" ] || { echo "$0: no such card image: $IMAGE" >&2; exit 1; }
[ -w "$IMAGE" ] || { echo "$0: $IMAGE is not writable, and this edits in place" >&2; exit 1; }
[ -f "$DOCUMENT" ] || { echo "$0: no such document: $DOCUMENT" >&2; exit 1; }

# The GPT header signature at LBA 1, exactly as run.sh checks it. A file that is
# not a card image would otherwise reach `sgdisk` and produce a confusing
# complaint about a partition number.
_sig=$(od -A n -c -j 512 -N 8 "$IMAGE" 2>/dev/null | tr -d ' \t\n')
if [ "$_sig" != "EFIPART" ]; then
    echo "$0: $IMAGE has no GPT header at LBA 1 (read '$_sig')." >&2
    exit 1
fi

# --- where /state is, read out of the image's own GPT -----------------------
#
# The partition NUMBER comes from rivet-core rather than from this file, and the
# LABEL is then asserted against the image. Either alone would be weaker: a
# number could silently address the wrong partition on an image built by a
# future layout, and a label search would not catch a layout in which /state
# moved. `ADR-011`, and the same belt-and-braces `tools/update/build.sh` uses.
STATE_PARTITION=$(sed -n 's/^pub const STATE_PARTITION: *u32 *= *\([0-9]*\);.*/\1/p' \
    "$REPO/rivet-core/src/state.rs" | head -1)
STATE_PARTLABEL=$(sed -n 's/^pub const STATE_PARTLABEL: *&str *= *"\([^"]*\)".*/\1/p' \
    "$REPO/rivet-core/src/state.rs" | head -1)
case $STATE_PARTITION in
    ''|*[!0-9]*)
        echo "$0: could not read STATE_PARTITION out of rivet-core. Refusing to guess." >&2
        exit 1 ;;
esac
[ -n "$STATE_PARTLABEL" ] || {
    echo "$0: could not read STATE_PARTLABEL out of rivet-core. Refusing to guess." >&2
    exit 1
}

INFO=$(sgdisk -i "$STATE_PARTITION" "$IMAGE")
LABEL=$(printf '%s\n' "$INFO" | sed -n "s/^Partition name: *'\(.*\)'.*/\1/p")
START=$(printf '%s\n' "$INFO" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
END=$(printf '%s\n' "$INFO" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')

if [ "$LABEL" != "$STATE_PARTLABEL" ]; then
    echo "$0: partition $STATE_PARTITION of $IMAGE is labelled '$LABEL', not" >&2
    echo "    '$STATE_PARTLABEL'. This image was not written by a cmd/rivet-image" >&2
    echo "    that agrees with this checkout, and writing to it would put a" >&2
    echo "    document into whatever that partition actually is." >&2
    exit 1
fi
case "$START$END" in
    ''|*[!0-9]*) echo "$0: could not read partition $STATE_PARTITION's extent" >&2; exit 1 ;;
esac
COUNT=$((END - START + 1))

# --- the paths the running system uses, read out of the crates that own them -
CONFIG_DIR=$(sed -n 's/^pub const CONFIG_DIR: *&str *= *"\([^"]*\)".*/\1/p' \
    "$REPO/rivet-core/src/state.rs" | head -1)
STATE_MOUNT_POINT=$(sed -n 's/^pub const STATE_MOUNT_POINT: *&str *= *"\([^"]*\)".*/\1/p' \
    "$REPO/rivet-core/src/state.rs" | head -1)
LAYER=$(sed -n 's/^pub const MACHINE_LAYER_FILE: *&str *= *"\([^"]*\)".*/\1/p' \
    "$REPO/rivet-store/src/store.rs" | head -1)
for v in "$CONFIG_DIR" "$STATE_MOUNT_POINT" "$LAYER"; do
    [ -n "$v" ] || { echo "$0: could not read the document's path out of the crates" >&2; exit 1; }
done
# CONFIG_DIR is absolute and rooted at the mount point; inside the filesystem
# image the same directory has no /state on the front.
INNER_DIR=${CONFIG_DIR#"$STATE_MOUNT_POINT"}

# **The modes are the board's, read off it on 2026-08-28** — /state/config is
# rwxr-xr-x and machine.toml is rw-r--r--, which is what PID 1's mkdir_p and
# write_file produce under its umask of 022. Set explicitly rather than left to
# debugfs's defaults, because a seed whose modes differ from a real boot's would
# make every mode assertion in a test about this harness instead of about Rivet.
DIR_MODE=040755
FILE_MODE=0100644

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
PART=$WORK/state.img

echo "seed-card.sh: $IMAGE"
echo "  /state is partition $STATE_PARTITION ('$LABEL'), sectors $START..$END ($((COUNT / 2048)) MiB)"
echo "  writing $DOCUMENT to $CONFIG_DIR/$LAYER"

dd if="$IMAGE" of="$PART" bs=512 skip="$START" count="$COUNT" status=none

# debugfs takes one command per line from a file. `mkdir` on an existing
# directory and `rm` on an absent file both complain and neither is a failure
# here, so their output is filtered rather than the script being made to care —
# what matters is the read-back below, which is the only thing that proves
# anything.
cat >"$WORK/cmds" <<EOF
mkdir $INNER_DIR
sif $INNER_DIR mode $DIR_MODE
rm $INNER_DIR/$LAYER
cd $INNER_DIR
write $DOCUMENT $LAYER
sif $LAYER mode $FILE_MODE
EOF

debugfs -w -f "$WORK/cmds" "$PART" 2>&1 \
    | grep -vE "^debugfs |^$|File exists|File not found|Allocated inode" || true

# **Read it back before writing it home.** `debugfs` reports success for a
# `write` that produced nothing more often than is comfortable, and a card that
# looks seeded and is not would send the next reader hunting a Rivet defect —
# which is precisely how §62.8's invalid experiment happened.
debugfs -R "cat $INNER_DIR/$LAYER" "$PART" 2>/dev/null >"$WORK/readback" || true
if ! cmp -s "$DOCUMENT" "$WORK/readback"; then
    echo "$0: the document did not survive the write into $PART." >&2
    echo "    Wrote $(wc -c <"$DOCUMENT") bytes, read back $(wc -c <"$WORK/readback")." >&2
    echo "    The card image has NOT been modified." >&2
    exit 1
fi

# Only now does the image change.
dd if="$PART" of="$IMAGE" bs=512 seek="$START" count="$COUNT" conv=notrunc status=none

echo "  verified: $(wc -c <"$WORK/readback") bytes read back from the image's own ext4"
echo
echo "This is a machine that has ALREADY been provisioned — boot 2, not boot 1."
echo "PID 1 will find a stored document and will NOT apply a provisioning file"
echo "over it (ADR-011). Boot it with:"
echo
echo "    ./tools/qemu/run.sh <init> --supervisor <sup> --card $IMAGE --card-rw"
echo
echo "and again, to ask what the second boot does differently."
