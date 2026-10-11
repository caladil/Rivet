#!/bin/sh
# Copy the target kernel and device tree off a running Raspberry Pi, so the
# loop runs the exact kernel the hardware runs rather than a generic arm64 one.
# ADR-026 §2.1 explains why that distinction matters.
#
# Usage: ./fetch-kernel.sh user@host
#
# ## The host this copies from is NOT a Rivet
#
# **It runs Raspberry Pi OS.** What is being copied is
# `/boot/firmware/kernel8.img`, which a Rivet does not have and never did. It is
# a BUILD INPUT — `cmd/rivet-image` puts that kernel on a card and
# `tools/container/build-card.sh` refuses without it — and the machine it comes
# from is somebody else's operating system, reached the way that operating
# system is reached. Nothing here is a route into a Rivet, and `ADR-035`'s
# ruling about Rivet's administration surface does not reach it.
#
# **There is a network-free route and it is not built here**: `mcopy -i
# <image>@@<offset> ::kernel8.img .` reads the file straight out of the FAT boot
# partition of a card image, with no network and no login. It is worth having
# the day somebody needs it; it is not worth guessing the offset today.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
TARGET=$1

if [ -z "$TARGET" ]; then
    echo "usage: $0 user@host    # a Raspberry Pi running the target kernel" >&2
    exit 2
fi

scp "$TARGET:/boot/firmware/kernel8.img" "$HERE/kernel8.img"
scp "$TARGET:/boot/firmware/bcm2711-rpi-4-b.dtb" "$HERE/bcm2711-rpi-4-b.dtb"

echo
echo "fetched from $TARGET:"
ls -la "$HERE/kernel8.img" "$HERE/bcm2711-rpi-4-b.dtb"
echo
# **Read out of the FILE that was just fetched, not asked of the host.** The
# two can differ — a host that has been updated and not rebooted reports the
# running kernel while the boot partition already holds the next one — and it
# is the file that ends up on a card.
echo "kernel version stamped in the image that was fetched:"
strings "$HERE/kernel8.img" 2>/dev/null | grep -m1 "^Linux version " \
    || echo "  (not found; \`strings\` may not be installed)"
echo
echo "Neither file is committed - see .gitignore and ADR-026 §6."
