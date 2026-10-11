#!/bin/sh
# Fetch the Raspberry Pi boot firmware the image builder needs.
#
# `ADR-010` §6 and `.gitignore` agree on why this is a fetch rather than a
# commit: the firmware is redistributable but is not this project's to carry.
# So it is pinned, verified, and downloaded — never vendored into git history.
#
# **Pinned by commit sha, not by tag.** A tag can be moved; a sha cannot. The
# tag it corresponds to is recorded below for humans.
set -e

# raspberrypi/firmware tag 1.20260521.
#
# Chosen because it is what the maintainer's own board reports running —
# `dmesg` on that Pi says "Attached to firmware from 2026-05-21T11:20:25", which
# matches this tag. So the image ships firmware already known to boot that
# hardware, rather than whatever happened to be newest.
FIRMWARE_SHA=09267f5354d40519d82fbd2193b9e211ec304055
FIRMWARE_TAG=1.20260521

# Only what a Pi 4 actually needs from the card. The first stage lives in the
# SPI EEPROM on this board (`ADR-002` §4), so there is no `bootcode.bin` here.
# sha256 of each file at the pinned sha — a download that does not match is
# refused rather than used (`AGENTS.md` §3).
FILES="start4.elf:3484b5ddc0f655e0e562680b7e2462ec21177763cc2d884b37d31e53b800bb02
fixup4.dat:c571fe712649b66409a4834b06af39dcef8dca6727e3abc10c6043f02028b5e5"

# `sha256sum` is GNU coreutils; macOS ships `shasum -a 256` instead, which
# prints the same `<hex>  <name>` line. Asked for by capability, not by OS.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d" " -f1
}

HERE=$(cd "$(dirname "$0")/.." && pwd)
DEST=$HERE/vendor/boot
mkdir -p "$DEST"

echo "raspberrypi/firmware $FIRMWARE_TAG ($FIRMWARE_SHA)"

for entry in $FILES; do
    name=${entry%%:*}
    want=${entry#*:}
    url="https://raw.githubusercontent.com/raspberrypi/firmware/$FIRMWARE_SHA/boot/$name"

    if [ -f "$DEST/$name" ]; then
        got=$(sha256 "$DEST/$name")
        if [ "$got" = "$want" ]; then
            echo "  $name  already present and verified"
            continue
        fi
        echo "  $name  present but does not match the pin; refetching" >&2
    fi

    code=$(curl -sL -o "$DEST/$name" -w "%{http_code}" "$url")
    if [ "$code" != "200" ]; then
        echo "$0: $name: HTTP $code from $url" >&2
        rm -f "$DEST/$name"
        exit 1
    fi

    got=$(sha256 "$DEST/$name")
    if [ "$got" != "$want" ]; then
        echo "$0: $name: sha256 mismatch" >&2
        echo "  expected $want" >&2
        echo "  got      $got" >&2
        echo "  Refusing it. Either the pin is wrong or the download is not what it claims." >&2
        rm -f "$DEST/$name"
        exit 1
    fi
    echo "  $name  fetched and verified ($(wc -c <"$DEST/$name" | tr -d " ") bytes)"
done

echo
echo "In $DEST — gitignored, per ADR-010 §6."
