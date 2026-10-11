#!/bin/sh
# Build a Rivet card image — and keep the symbols.
#
# **This is the first committed recipe for building a card.** Until 2026-08-24
# every card in `out/` was produced by somebody's shell history, which is the
# thing `tools/buildroot/build.sh` and `tools/nushell/build.sh` both exist to
# stop. `ADR-026` §6 is the general argument: a build that lives in one
# person's scrollback is a build nobody else can reproduce and nobody can
# review.
#
# Usage:
#   ./build.sh [--debug] [--device-mib N] [--default-slot a|b]
#
# Options:
#   --debug          also install gdbserver and strace — ADR-005 §7.4, ruled
#                    2026-08-24: a debugger reaches a card on a `--debug` build
#                    and on no other. Needs tools/debug/build.sh to have run.
#   --device-mib N   passed through to cmd/rivet-image
#   --default-slot   passed through
#
# Environment:
#   RIVET_BR_OUTPUT   Buildroot's output tree, for the vendored C mechanisms
#                       and the cross `strip`. Default: $HOME/.rivet/buildroot-<pin>
#   RIVET_NU          the nu binary. Default: $HOME/.rivet/nushell/bin/nu
#   RIVET_IDENTITY    a directory holding this device's TLS identity, as
#                       `cargo rivet setup` writes it: device.seed, device.crt,
#                       and a `peers` file listing the Ed25519 public keys
#                       allowed to reach the control plane.
#                       OPTIONAL, and the option almost nobody wants. Without
#                       one the card boots and rivet-mcpd refuses to start --
#                       which costs no other service and costs you the ONLY WAY
#                       IN. Since 2026-09-05 this script says so out loud
#                       instead of building that card in silence.
#                       ADR-019 §2 -- a machine arrives enrolled, and its
#                       credentials reach the card from the operator's own
#                       tooling rather than over a network.
#   RIVET_STAGING     the staging bundle rivet-provision writes beside the
#                     document: the certificates, operator credential and
#                     one-record register this machine's APPLICATIONS are handed
#                     before they first start (ADR-049). Optional, and most
#                     cards need none.
#   RIVET_APPLICATIONS  a SPACE-SEPARATED list of signed .rvt payloads this card
#                     CARRIES -- ADR-050 §4. Each rides on the boot partition;
#                     PID 1 lands it in /state/carried on the first boot and the
#                     supervisor asks rivet-deploy to install it. This is what
#                     makes SPEC-05 §1.1 true: `fleetd` comes up running because
#                     the card brought it.
#                     Each is VERIFIED here against the trust store this image
#                     will carry, so a payload this card would refuse is refused
#                     on your workstation. A build asked to carry payloads with
#                     no application key REFUSES rather than warning -- ADR-050
#                     §4.3, which is the failure otherwise found at 11pm.
#                     Optional, and most cards carry none.
#   RIVET_FLEET_REPOSITORY  a directory `cargo rivet fleet publish` has filed
#                     objects under. Given TOGETHER WITH RIVET_DOCUMENT,
#                     cmd/rivet-image reads back which application services the
#                     composed document already enables
#                     (rivet_config::Document::carried_services -- ADR-050
#                     §5.1's table: a server's document enables rivet-fleetd and
#                     rivet-proxyd, a kiosk's enables its auto-run application)
#                     and resolves each by digest out of this repository --
#                     additively with RIVET_APPLICATIONS, and on the same
#                     entitlement and trust-store terms. This is what makes a
#                     fresh card for a fleet-server role carry rivet-fleetd
#                     without anybody naming it (PLAN-Feature-Set-2026-09-12
#                     §4). REFUSED without RIVET_DOCUMENT -- there is no
#                     composed document to read a role's required services off.
#                     Optional, and most cards name none.
#   RIVET_DOCUMENT    the first-boot provisioning document. WITHOUT ONE THE
#                       CARD BOOTS AND NOTHING CAN REACH IT -- see
#                       tools/card/provisioning.example.toml
#   RIVET_DESCRIBE    the commit string that goes in the image's NAME.
#                       **Take this from WINDOWS git, not WSL's** — see below,
#                       and prefer `tools/card/bump.sh`, which mints the version
#                       and prints the describe for exactly this (ADR-027 §10).
#                       A `-dirty` describe is REFUSED.
#
# The image's VERSION is not an environment variable. `ADR-027` §10 makes
# `Cargo.toml`'s `[workspace.package]` version the single source and this script
# reads it; `tools/card/bump.sh` is what moves it.
#   RIVET_KEEP        how many card images to leave in out/. Default: 2 — this
#                       one and the one it replaces, which is what an A/B
#                       rollback needs. Older cards and their out/debug/ symbol
#                       trees are removed together after a successful build.
#                       RIVET_KEEP=0 keeps everything; see the note at the foot
#                       of this file for why the default is not 0.
#
# ## Why this script exists rather than a one-liner: the symbols
#
# `ADR-005` §7.5, ruled 2026-08-24: **keep an unstripped copy off the card.**
#
# `[profile.release]` used to set `strip = true`, which stripped the only
# artifact `cargo` produces — so a released board could be attached to and gave
# `0x22d0d0 in ?? ()`, and there was nothing anywhere to resolve it against.
# The profile now keeps symbols and DWARF, and **this script strips on the way
# onto the image**, so:
#
#   * the card carries exactly what it carried before — `ADR-018` §4's slots
#     and §10's distributed artifact are sized for stripped binaries and this
#     ruling must not grow either;
#   * `$UNSTRIPPED` keeps a copy of precisely those builds, which is what a
#     `gdb` on the development host resolves symbols against.
#
# **The strip is not optional and there is no flag to skip it.** A card built
# with symbols would be a quietly larger card, and `ADR-027` §3.2.1 makes an
# artifact's name a claim about what is in it.
#
# ## The part that is NOT solved here, and you should know before relying on it
#
# `ADR-005` §7.5.2 item 3: **nothing ties the retained copy to the card.**
# `$UNSTRIPPED` is a directory of binaries named after the build that made
# them, and if you build twice and flash the first card, `gdb` will happily
# resolve symbols from the second and be wrong about every address without
# saying so. `ADR-027` has no concept of a build *output* yet, so there is no
# named, signed artifact to point at. **Until there is: keep one card and one
# `$UNSTRIPPED` in play at a time.**
#
# ## Why the strip is here and not in `cmd/rivet-image`
#
# Stripping an aarch64 ELF needs the cross toolchain. Measured on 2026-08-24:
# the host's GNU `strip` 2.47 answers *"Unable to recognise the architecture of
# the input file"*. `ADR-009` §6 keeps the toolchain on Buildroot's side and
# has the image builder take **finished binaries** as inputs — nothing about
# how one was built is that crate's business — so teaching it to strip would
# put a cross-toolchain dependency inside the one crate deliberately ignorant
# of it. `tools/` is where that knowledge already lives.
set -e

# ---------------------------------------------------------------------------
# Where the time goes -- added 2026-08-30
# ---------------------------------------------------------------------------
#
# **Because two hours of this were spent in one day and nobody could say on
# what.** A build that takes ten minutes and reports nothing about them is one
# whose cost can only be guessed at, and every guess about where a build spends
# its time is wrong at least once. Every run now ends with a breakdown, so the
# question is answered by the build rather than by an afternoon of instrumenting
# it later.
#
# `date +%s` rather than anything richer: it is in every shell this runs in, and
# whole seconds are the resolution the question has -- a phase under a second is
# not why a build takes ten minutes.
PHASE_LOG=""
PHASE_CURRENT=""
PHASE_BEGAN=$(date +%s)
BUILD_BEGAN=$PHASE_BEGAN

# Close the running phase and open one named $1. With no argument it just closes
# the last one, which is what `report_phases` does.
phase() {
    _now=$(date +%s)
    if [ -n "$PHASE_CURRENT" ]; then
        PHASE_LOG="$PHASE_LOG$((_now - PHASE_BEGAN)) $PHASE_CURRENT
"
    fi
    PHASE_CURRENT=${1:-}
    PHASE_BEGAN=$_now
}

# **A card this build would produce that is quietly worse than the one asked
# for** — `PLAN.md` §123, Wave 4.3.
#
# Two paths used to print a `note:` and carry on: a missing `fdtoverlay` or
# overlay, which produces a card with no `/dev/dri` at all, and a missing
# Buildroot kernel, which falls back to `tools/qemu/kernel8.img` — a Raspberry
# Pi OS kernel fetched by `scp` with no digest, whose wireless stack is modular
# and which a Rivet image cannot load a module for.
#
# Both produce a card with a **normal name**. It is written, it boots, and the
# defect is discovered later against something that looks like a different
# problem: a kiosk application that will not draw, or wireless that never
# associates. The note went by in a page of build output nobody re-reads.
#
# So a degradation now REFUSES, and the way to accept it is to say so:
#
#   RIVET_ACCEPT_DEGRADED=1 tools/card/build.sh …
#
# which is not a hurdle — it is the difference between a card somebody chose and
# a card that happened.
degraded() {
    echo >&2
    echo "REFUSED: this build would produce a card that is quietly worse than the" >&2
    echo "  one you asked for, and it would be named as if it were not." >&2
    for _line in "$@"; do
        echo "    $_line" >&2
    done
    if [ "${RIVET_ACCEPT_DEGRADED:-}" = 1 ]; then
        echo "  RIVET_ACCEPT_DEGRADED=1 — continuing, and the card WILL have this." >&2
        echo >&2
        return 0
    fi
    echo "  Fix it, or say RIVET_ACCEPT_DEGRADED=1 to build this card deliberately." >&2
    exit 1
}

# Slowest last, so the eye lands on what to fix.
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

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
cd "$REPO"

BR_VERSION=2026.02.3
BR_OUTPUT=${RIVET_BR_OUTPUT:-$HOME/.rivet/buildroot-$BR_VERSION}
NU=${RIVET_NU:-$HOME/.rivet/nushell/bin/nu}
TARGET=aarch64-unknown-linux-musl
# **`ADR-027` §3.1's token for that triple**, which the image records at
# `/system/arch` and every artifact is compared against on the machine
# (`PLAN-Feature-Set-2026-09-12` §3.3g). Stated here, beside the triple it
# names, because this script is what chose the target: the compiler on the
# machine would say `arm` where ADR-027 says `armv6`, so the machine cannot work
# it out for itself. `rivet_remote::name::arch_for` is the same table for
# `cargo rivet package`.
case "$TARGET" in
    aarch64-unknown-linux-musl) ARCH=aarch64 ;;
    *) echo "build.sh: no ADR-027 architecture token is known for $TARGET" >&2; exit 1 ;;
esac
STRIP="$BR_OUTPUT/host/bin/aarch64-linux-strip"

# The cross C compiler, which Rivet's own binaries needed none of until
# 2026-08-28. `rivetos-tls` links `ring`, whose build compiles C and assembly,
# so `cc-rs` has to be told what to use -- without this it looks for
# `aarch64-linux-musl-gcc` on PATH and the build fails at rivetos-tls with
# `error occurred in cc-rs: failed to find tool`.
#
# ADR-009 §3.7.4 prices this and does not hide it: adopting `ring` moved Rivet
# from "no C anywhere in its own binaries" to "one C dependency, in the newest
# and most security-sensitive one". The compiler itself is not new -- Buildroot
# has produced it since Stage 0, beside the `strip` above -- so what this costs
# is this variable and nothing else.
CROSS_CC="$BR_OUTPUT/host/bin/aarch64-linux-gcc"
export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-$CROSS_CC}"

DEBUG=no
PASSTHROUGH=""
while [ $# -gt 0 ]; do
    case "$1" in
        --debug) DEBUG=yes ;;
        --device-mib|--default-slot)
            [ $# -ge 2 ] || { echo "$1 needs a value" >&2; exit 2; }
            PASSTHROUGH="$PASSTHROUGH $1 $2"
            shift
            ;;
        -h|--help) sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
    shift
done

# The version string goes in the image's NAME, and `ADR-027` §3.2.1 makes
# `-dirty` a claim about uncommitted changes. `AGENTS.md` §10.5: WSL's git
# reads this CRLF working copy as modified and would stamp `-dirty` on every
# build for ever, asserting something that is not true and worse for being
# permanent. So it is taken from the caller, who is expected to have asked
# Windows git.
DESCRIBE=${RIVET_DESCRIBE:-}
if [ -z "$DESCRIBE" ]; then
    cat >&2 <<EOF
tools/card/build.sh: RIVET_DESCRIBE is not set.

The image's name carries it and ADR-027 §3.2.1 makes that name a claim. Take it
from WINDOWS git, not from WSL's — WSL reads this working copy as dirty because
of line endings (AGENTS.md §10.5), so a name built here would say `-dirty` on
every build for ever.

    RIVET_DESCRIBE=\$(tools/card/bump.sh | tail -1) tools/card/build.sh
EOF
    exit 2
fi

# ADR-027 §10.2 step 1: refuse a dirty tree. The caller already asked Windows
# git, which is the side that can tell (AGENTS.md §10.5), so the answer is
# sitting in the string it handed over and no second probe is needed. There is
# no override flag — §10.2 rules the sequence, and a `-dirty` image is exactly
# the artifact §3.2.1's naming rule exists to keep out of `out/`.
case "$DESCRIBE" in
    *-dirty)
        cat >&2 <<EOF
tools/card/build.sh: RIVET_DESCRIBE is '$DESCRIBE' — the tree is dirty.

ADR-027 §10.2 refuses to build from one. The version and the commit are a 1:1
mapping, and an image named for a working copy nobody can check out again is a
claim about bytes that cannot be reproduced.

Commit what is outstanding, then mint the version:

    RIVET_DESCRIBE=\$(tools/card/bump.sh | tail -1) tools/card/build.sh
EOF
        exit 1
        ;;
esac

# ADR-027 §10: Cargo.toml's [workspace.package] version is the SINGLE source of
# the version, and this script READS it. It used to state `0.0.1` itself, in two
# places, and agree with Cargo.toml by luck — one fact with two spellings and no
# vocabulary for their disagreement, which is ADR-011 §3.
VERSION=$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^version *= *"\([^"]*\)".*/\1/p' "$REPO/Cargo.toml" | head -1)
case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *)
        echo "tools/card/build.sh: no [workspace.package] version in Cargo.toml (read '$VERSION')" >&2
        exit 1
        ;;
esac

if [ ! -x "$CC_aarch64_unknown_linux_musl" ]; then
    cat >&2 <<EOF
tools/card/build.sh: no cross C compiler at $CC_aarch64_unknown_linux_musl

rivetos-tls links ring (ADR-009 §3.7), whose build compiles C and assembly, so
this build needs one. Buildroot produces it beside the cross strip; run
tools/buildroot/build.sh, or point RIVET_BR_OUTPUT at an existing output tree,
or set CC_aarch64_unknown_linux_musl yourself.

Refusing here rather than letting cargo fail deep inside a dependency's build
script, where the message is about cc-rs and not about this.
EOF
    exit 1
fi

if [ ! -x "$STRIP" ]; then
    cat >&2 <<EOF
tools/card/build.sh: no cross strip at $STRIP

ADR-005 §7.5 keeps symbols in the build and strips on the way onto the card, and
the host's strip cannot read an aarch64 ELF. Build the toolchain:

    tools/buildroot/build.sh --fetch toolchain

or point RIVET_BR_OUTPUT at an existing output tree.
EOF
    exit 1
fi

# git, on ADR-009 §6's Tier B terms exactly. **The one binary**, not
# the git-core directory beside it: 171 entries of which 147 are hardlinks to
# this same inode, and every verb Rivet needs is a builtin reached as
# `git <verb>` (PLAN.md §43.6).
GIT="$BR_OUTPUT/target/usr/bin/git"
[ -f "$GIT" ] || { echo "no git at $GIT — tools/buildroot/build.sh (BR2_PACKAGE_GIT)" >&2; exit 1; }

# git-http-backend, ADR-035 §4's smart HTTP. **A SECOND FILE and not another
# name for the one above**: PLAN.md §84.1 measured the inodes in this very tree
# and found `git` and `git-upload-pack` share one while this does not. It lives
# in the git-core directory that §43.6's note says is otherwise 171 hardlinks to
# `git` -- which is true of the builtins and was NOT true of this, so the card
# answered "not a git command" until it was carried.
HTTP_BACKEND="$BR_OUTPUT/target/usr/libexec/git-core/git-http-backend"
[ -f "$HTTP_BACKEND" ] || {
    echo "no git-http-backend at $HTTP_BACKEND" >&2
    echo "  It is part of BR2_PACKAGE_GIT and is built by default; if it is missing," >&2
    echo "  the git package needs rebuilding rather than reconfiguring." >&2
    exit 1
}

# ---------------------------------------------------------------------------
# The kernel -- ADR-024 §5, wired 2026-08-30
# ---------------------------------------------------------------------------
#
# **Buildroot's, and until today it was not.** cmd/rivet-image fell back to
# tools/qemu/kernel8.img, which fetch-kernel.sh copies off a running Raspberry
# Pi OS board -- so every Rivet card ever built booted somebody else's kernel
# rather than the one ADR-024 §5 rules Buildroot builds.
#
# The cost was measured on the board (PLAN.md §102.25): Pi OS's kernel has a
# MODULAR wireless stack, a Rivet image has no /lib/modules, and
# /sys/bus/sdio/drivers answered `entry_count: 0` -- no driver on the bus at
# all. Every claim in this repository about what is compiled in was describing
# a kernel the boards do not run.
#
# The two go together and cmd/rivet-image refuses half a pair: a device tree
# from one build beside an image from another boots, and then disagrees with
# itself about the hardware.
# ---------------------------------------------------------------------------
# The display controller, merged INTO the device tree -- ruled 2026-08-31
# ---------------------------------------------------------------------------
#
# **The kernel has the drivers and nothing binds without a device.** DRM, VC4
# and V3D are built in (`tools/buildroot/linux/rivet.config`), and a board
# answered with `v3d`, `vc4-drm`, `vc4_crtc` and `vc4_dpi` on
# /sys/bus/platform/drivers -- beside an empty /sys/class/drm, because the Pi 4's
# display controller is `status = "disabled"` in the device tree. `PLAN.md` §104.8.
#
# Stock Raspberry Pi OS solves this with `dtoverlay=vc4-kms-v3d-pi4` in
# config.txt and an `overlays/` directory on the boot partition. **The
# maintainer ruled the other way on 2026-08-31: merge it at BUILD time.**
#
# The argument is this project's own rather than a preference.
# `cmd/rivet-image`'s `BootFile` is a flat name/bytes pair and its `check_name`
# refuses a `/`, so shipping the overlay would mean teaching the FAT writer
# about directories for a file the firmware then has to find by its own rules.
# Merging produces ONE device tree that already describes the hardware, which is
# what this project does everywhere else -- and, like every other input, the
# result is read back and checked rather than trusted.
#
# **`vc4-kms-v3d-pi4` and not `vc4-kms-v3d`**: the base tree says
# `compatible = "raspberrypi,4-model-b"`, and the Pi kernel's overlay README
# describes the `-pi4` variant as the one for that board.
#
# **/dev/fb0 survives this**, checked before it was attempted:
# `CONFIG_DRM_FBDEV_EMULATION=y`, so VC4 provides the framebuffer node as well
# as /dev/dri/card0. The splash and `ADR-015` §6.2's display grant keep working.
merge_display_overlay() {
    _base=$1
    # Buildroot does not install overlays to images/, so this reaches into the
    # kernel build directory. Named rather than hidden: it is the one place this
    # script touches Buildroot's internals, and it is why the two misses below
    # are warnings rather than failures.
    _overlay="$BR_OUTPUT/build/linux-custom/arch/arm64/boot/dts/overlays/vc4-kms-v3d-pi4.dtbo"
    _merged=$(mktemp -d)/bcm2711-rpi-4-b.dtb

    if ! command -v fdtoverlay >/dev/null 2>&1; then
        degraded "no fdtoverlay, so the device tree is UNMERGED and this card has no" \
                 "/dev/dri: no GPU, and no display for a kiosk application." \
                 "fdtoverlay ships with dtc. PLAN.md §104.8."
        printf '%s' "$_base"; return 0
    fi
    if [ ! -f "$_overlay" ]; then
        degraded "no $_overlay, so the device tree is UNMERGED and this card has no" \
                 "/dev/dri: no GPU, and no display for a kiosk application." \
                 "Build the kernel with tools/container/build-buildroot.sh."
        printf '%s' "$_base"; return 0
    fi

    if ! fdtoverlay -i "$_base" -o "$_merged" "$_overlay" >&2; then
        echo "REFUSED: fdtoverlay could not merge $_overlay into the device tree." >&2
        echo "    A card whose device tree silently lost the display controller is" >&2
        echo "    the silent wrong answer this build exists to refuse." >&2
        exit 1
    fi

    # **Verify, do not assume.** `fdtoverlay` exits 0 for a merge that changed
    # nothing useful -- an overlay whose fixups resolve no symbols still writes
    # an output tree. `tools/buildroot/build.sh --show-config` learned the same
    # lesson about Kconfig on the same day, one level out: a guard that reports
    # a stale or empty reading is worse than none, because it is believed.
    #
    # The check is the V3D block's own `status`, which the base tree carries as
    # `disabled` and the merged one as `okay` -- measured on both, rather than
    # inferred from the overlay's name. `brcm,2711-v3d` is its compatible
    # string and is deliberately not the `bcm2711-` spelling every neighbouring
    # node uses.
    if ! dtc -I dtb -O dts "$_merged" 2>/dev/null \
        | awk '/brcm,2711-v3d/,/}/' | grep -q 'status = "okay"'; then
        echo "REFUSED: the merged device tree still has the V3D block disabled." >&2
        echo "    fdtoverlay reported success and the result describes no GPU," >&2
        echo "    which is a card that boots and has no /dev/dri. Nothing was written." >&2
        exit 1
    fi
    echo "--- device tree: merged $(basename "$_overlay"); V3D is now okay" >&2
    printf '%s' "$_merged"
}

# ---------------------------------------------------------------------------
# Mesa's shared libraries, and the closure is COMPUTED -- 2026-08-31
# ---------------------------------------------------------------------------
#
# `PLAN.md` §104.17. Mesa is the first thing on a Rivet that cannot be static:
# `package/mesa3d/Config.in` has `depends on !BR2_STATIC_LIBS` and there is no
# static libEGL to ask for. So a card that wants a GPU ships shared objects,
# and this is what decides WHICH.
#
# **The set is walked, not listed.** Naming the libraries by hand would be a
# list that is right today: `libEGL` needs `libgallium`, which needs
# `libstdc++` and `libgcc_s`, and a Mesa upgrade that added a dependency would
# produce a card whose GPU fails at the FIRST dlopen with a message about a
# missing SONAME -- on the board, at boot, on a machine with no shell. So this
# reads `NEEDED` out of the ELF files and follows it, which is the same
# discipline `merge_display_overlay` applies to the device tree one file over.
#
# **Named by SONAME, resolved with `cp -L`.** A `NEEDED` entry names
# `libEGL.so.1`; Buildroot ships that as a symlink to `libEGL.so.1.0.0`.
# Copying with `-L` and keeping the LINK's name writes one regular file called
# `libEGL.so.1`, which is what the loader opens. That is how this image has no
# symlinks in `/lib` and needs none.
#
# **`libc.so` is excluded and is not missing.** On musl the C library and the
# loader are one file, and it is already installed as
# `/lib/ld-musl-aarch64.so.1`. Shipping it a second time under its other name
# would put two copies of the same 600 KiB on a read-only partition.
# The first directory in `$2` holding `$1`, or nothing.
find_library() {
    for _d in $2; do
        [ -e "$_d/$1" ] && { printf '%s' "$_d/$1"; return 0; }
    done
    return 0
}

# The shared libraries the VENDORED binaries ask for — none, on a tree where
# they all chose static.
#
# `libc.so` is skipped for `collect_libraries`'s own reason, twenty lines down:
# on musl the loader IS libc, and it is installed as `/lib/ld-musl-aarch64.so.1`
# whether or not anything asks for it.
vendored_needed() {
    for _vb in "$GIT" "$HTTP_BACKEND" "$SUPPLICANT" "$WPA_CLI" "$UDHCPC" "$NU"; do
        [ -f "$_vb" ] || continue
        # `|| true` because a STATIC binary is the ordinary case and makes
        # `grep` exit 1, which `set -e` would otherwise treat as a failure of
        # the build rather than as the good news it is.
        readelf -d "$_vb" 2>/dev/null \
            | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' \
            | grep -v '^libc\.so$' || true
    done
    return 0
}

collect_libraries() {
    _out="$1"
    # **Two places, in this order, and the second is not a fallback for the
    # first being broken.** Buildroot builds Mesa into `target/usr/lib`; the
    # C++ runtime it links against is the TOOLCHAIN's and Buildroot does not
    # stage it, so `libstdc++.so.6` and `libgcc_s.so.1` are only ever in the
    # cross sysroot. `$CROSS_SYSROOT` is already where the loader comes from
    # twenty lines above, for the same reason.
    #
    # This was found by the walker refusing, which is the guard working: the
    # first attempt searched `target/usr/lib` alone and stopped with
    # "libstdc++.so.6 is NEEDED by a library this card ships and is not in …"
    # rather than writing a card whose GPU fails at the first dlopen.
    _dirs="$BR_OUTPUT/target/usr/lib $BR_OUTPUT/target/lib"
    [ -n "$CROSS_SYSROOT" ] && _dirs="$_dirs $CROSS_SYSROOT/usr/lib $CROSS_SYSROOT/lib"
    # **Two sets of roots, and the second was added 2026-09-20.**
    #
    # Mesa's three are the GPU's. The rest are COMPUTED from the vendored
    # binaries this card carries, because `BR2_SHARED_STATIC_LIBS=y` lets every
    # Buildroot package choose for itself and they do not all choose the same
    # thing twice. On the tree built that morning `git` chose shared and wanted
    # `libz.so.1`, where the same defconfig had produced a static `git` on
    # 2026-09-03.
    #
    # The defconfig predicted precisely this — *"the next package added, or the
    # next version of one of these, can change any of those five lines without
    # anything saying so"* — so the roots are derived rather than listed, and the
    # next package to flip needs no edit here. Everything else is reached from
    # these by the walk below.
    _todo=""
    _gpu="libEGL.so.1 libGLESv2.so.2 libgbm.so.1"
    for _r in $_gpu; do
        [ -n "$(find_library "$_r" "$_dirs")" ] || {
            echo "note: $_r is absent, so this card has NO GPU LIBRARIES." >&2
            echo "      Mesa is BR2_PACKAGE_MESA3D in the Buildroot defconfig." >&2
            _gpu=""
            break
        }
    done
    # **The compositor's one library** — `ADR-058` §5.1, added 2026-09-20.
    #
    # `cmd/rivet-displayd` is built on Smithay, which depends on the
    # `xkbcommon` crate unconditionally, so `libxkbcommon.so.0` is a `NEEDED`
    # entry of a binary this card ships. It is NAMED here rather than derived
    # the way `vendored_needed` derives its roots, and the reason is ordering
    # and nothing better: this function runs before `cargo build`, so the
    # binary that wants it does not exist yet.
    #
    # Kept out of `$_gpu` deliberately. That list is all-or-nothing — one
    # missing member means "this card has no GPU" — and a tree built before
    # `BR2_PACKAGE_LIBXKBCOMMON` was added would otherwise silently drop Mesa
    # as well. Here a missing library costs the compositor and says so.
    _compositor="libxkbcommon.so.0"
    if [ -z "$(find_library "$_compositor" "$_dirs")" ]; then
        echo "note: $_compositor is absent, so rivet-displayd cannot run on this card." >&2
        echo "      It is BR2_PACKAGE_LIBXKBCOMMON in the Buildroot defconfig;" >&2
        echo "      the build will refuse later, at must_be_runnable." >&2
        _compositor=""
    fi

    _todo="$_gpu $_compositor $(vendored_needed | sort -u | tr '\n' ' ')"

    # Nothing asked for anything: a fully static card, which is the good case
    # and was every card before Mesa.
    [ -n "$(printf '%s' "$_todo" | tr -d ' ')" ] || return 0

    mkdir -p "$_out"
    _seen=""
    while [ -n "$_todo" ]; do
        _next=""
        for _name in $_todo; do
            case " $_seen " in *" $_name "*) continue ;; esac
            _seen="$_seen $_name"
            _found=$(find_library "$_name" "$_dirs")
            [ -n "$_found" ] || {
                echo "REFUSED: $_name is NEEDED by a library this card ships and is in none of" >&2
                echo "    $_dirs" >&2
                echo "    A card written now would boot and fail at the first dlopen, on a" >&2
                echo "    machine with no shell to diagnose it. Nothing was written." >&2
                exit 1
            }
            # `-L` resolves the symlink; the destination keeps the SONAME.
            cp -L "$_found" "$_out/$_name"
            for _dep in $(readelf -d "$_out/$_name" 2>/dev/null \
                | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
                # musl's libc IS the loader, already installed. See above.
                [ "$_dep" = "libc.so" ] && continue
                _next="$_next $_dep"
            done
        done
        _todo="$_next"
    done
    printf '%s' "$_seen"
}

# **Mesa's GBM backend, which the closure walker cannot see.**
#
# `libgbm` `dlopen`s its backend from a directory it has COMPILED IN --
# `strings libgbm.so.1` contains `/usr/lib/gbm` and it looks nowhere else. A
# dlopen is not a NEEDED entry, so `collect_libraries` walked straight past
# `dri_gbm.so` and produced a card whose eight libraries were all present and
# whose `gbm_create_device` returned NULL on the board.
#
# That is this file's own lesson from `merge_display_overlay` in a third place:
# a check that passes is not the same as a thing that works. The closure was
# complete and the answer was still wrong, because the question -- "what does
# this binary NEED" -- was narrower than the machine's behaviour.
#
# It prints the backends' own NEEDED entries so the caller can add them to the
# /lib closure; on this build they are all libraries the roots already pulled.
collect_gbm_backends() {
    _src="$BR_OUTPUT/target/usr/lib/gbm"
    _out="$1"
    [ -d "$_src" ] || return 0
    mkdir -p "$_out"
    for _b in "$_src"/*.so; do
        [ -e "$_b" ] || continue
        cp -L "$_b" "$_out/$(basename "$_b")"
        printf '%s ' "$(basename "$_b")"
    done
}

phase "locating inputs"
KERNEL_IMAGE="$BR_OUTPUT/images/Image"
KERNEL_DTB="$BR_OUTPUT/images/bcm2711-rpi-4-b.dtb"
KERNEL_ARGS=""
if [ -f "$KERNEL_IMAGE" ] && [ -f "$KERNEL_DTB" ]; then
    KERNEL_DTB=$(merge_display_overlay "$KERNEL_DTB")
    KERNEL_ARGS="--kernel $KERNEL_IMAGE --device-tree $KERNEL_DTB"
else
    degraded "no kernel in $BR_OUTPUT/images -- falling back to tools/qemu/kernel8.img," \
             "which came off a Raspberry Pi OS board and was fetched by scp with no digest." \
             "That kernel's wireless stack is modular and a Rivet image cannot load it." \
             "Build one with tools/container/build-buildroot.sh. PLAN.md §102.25."
fi

# ---------------------------------------------------------------------------
# Wireless -- ADR-004, added 2026-08-30
# ---------------------------------------------------------------------------
#
# **All four are OPTIONAL and none of them refuses.** A card with no wireless is
# a supported machine: ADR-004 §2 brings up wired first and §2.4 keeps the wired
# interface afterwards, and every Rivet board so far has run on a static
# address. That is the opposite of `git` above, whose absence IS a fault.
#
# What is not optional is honesty about which card you have, so each one says
# whether it was found -- and the verification block near the end checks that
# whatever was passed actually landed. A silent omission here would produce a
# card that looks like the last one and has no wireless.

# wpa_supplicant -- ADR-004 §3's permanent Tier C dependency.
SUPPLICANT="$BR_OUTPUT/target/usr/sbin/wpa_supplicant"
SUPPLICANT_ARG=""
if [ -f "$SUPPLICANT" ]; then SUPPLICANT_ARG="--supplicant $SUPPLICANT"; fi

# wpa_cli -- a BRING-UP tool. It proves the C side associates independently of
# rivet-netd, so a failure has one candidate cause instead of two.
WPA_CLI="$BR_OUTPUT/target/usr/sbin/wpa_cli"
WPA_CLI_ARG=""
if [ -f "$WPA_CLI" ]; then WPA_CLI_ARG="--wpa-cli $WPA_CLI"; fi

# udhcpc -- BusyBox, built with exactly ONE applet
# (tools/buildroot/busybox/busybox.config). It is installed on the card as
# `udhcpc` because that is what it is; see system::UDHCPC_PATH.
UDHCPC="$BR_OUTPUT/target/bin/busybox"
UDHCPC_ARG=""
if [ -f "$UDHCPC" ]; then UDHCPC_ARG="--udhcpc $UDHCPC"; fi
# The program udhcpc execs is one of $OURS and lives in $STAGED, which does not
# exist yet -- so DHCP_HOOK_ARG is composed further down, beside SPLASH_ARG.

# The CYW43455 firmware. A DIRECTORY: cmd/rivet-image knows the three names,
# which ADR-004 §8.1 measured off a real board. They are SYMLINKS into
# cypress/ here and land on the card as real files, because a copied node is
# read with fs::read.
WIFI_FIRMWARE="$BR_OUTPUT/target/lib/firmware/brcm"
WIFI_FIRMWARE_ARG=""
if [ -d "$WIFI_FIRMWARE" ]; then WIFI_FIRMWARE_ARG="--wifi-firmware $WIFI_FIRMWARE"; fi

# ---------------------------------------------------------------------------
# The dynamic loader -- Shared Rivets, added 2026-08-31
# ---------------------------------------------------------------------------
#
# **On musl the loader IS the C library.** `libc.so` in the sysroot is the file,
# and `ld-musl-aarch64.so.1` beside it is a symlink to it; cmd/rivet-image reads
# the source with fs::read, which follows the link, so either path would work
# and the real file is named to make that not matter.
#
# **This changes nothing about Rivet's own binaries.** Every one of them is
# statically linked (ADR-003 §5) and none of them looks at this file. It is on
# the card so that an APPLICATION can be dynamic, which is what Shared Rivets
# needs; see cmd/rivet-image's system::LOADER_PATH.
#
# OPTIONAL, on wireless's terms rather than git's: a card with no loader boots,
# converges and is reachable exactly as every card before today did. What it
# cannot do is run a dynamic application -- and the INSTALLER is what says so,
# because an application declaring a closure is refused before it is unpacked.
#
# The sysroot is asked for rather than guessed: BR2_TOOLCHAIN_EXTERNAL_BOOTLIN
# puts the real toolchain under host/opt/ext-toolchain and leaves symlinks in
# host/bin, so a hard-coded host/<tuple>/sysroot is one Buildroot configuration
# change away from being wrong (tools/container/Dockerfile says as much).
LOADER=""
LOADER_ARG=""
CROSS_GCC="$BR_OUTPUT/host/bin/aarch64-linux-gcc"
if [ -x "$CROSS_GCC" ]; then
    CROSS_SYSROOT=$("$CROSS_GCC" -print-sysroot 2>/dev/null || true)
    if [ -n "$CROSS_SYSROOT" ] && [ -f "$CROSS_SYSROOT/lib/libc.so" ]; then
        LOADER="$CROSS_SYSROOT/lib/libc.so"
        LOADER_ARG="--loader $LOADER"
    fi
fi
if [ -n "$LOADER_ARG" ]; then
    echo "--- loader: $LOADER -> /lib/ld-musl-aarch64.so.1"
else
    echo "note: no musl loader found; this card runs static applications only" >&2
fi

# **The GPU's libraries** -- see `collect_libraries`. Only meaningful when there
# is a loader, which is why it is here rather than beside the kernel: a library
# on a card that cannot run a dynamic program is 17 MB nothing will open.
LIBRARY_ARGS=""
if [ -n "$LOADER_ARG" ]; then
    LIB_STAGE=$(mktemp -d)/lib
    SHIPPED=$(collect_libraries "$LIB_STAGE")
    if [ -n "$SHIPPED" ]; then
        for _l in $SHIPPED; do
            LIBRARY_ARGS="$LIBRARY_ARGS --library $LIB_STAGE/$_l"
        done
        echo "--- /lib:$(printf '%s' "$SHIPPED" | wc -w) shared libraries ->$SHIPPED"
    fi
    GBM_STAGE=$(mktemp -d)/gbm
    BACKENDS=$(collect_gbm_backends "$GBM_STAGE")
    for _b in $BACKENDS; do
        LIBRARY_ARGS="$LIBRARY_ARGS --library-at /usr/lib/gbm/$_b $GBM_STAGE/$_b"
    done
    [ -n "$BACKENDS" ] && echo "--- /usr/lib/gbm: $BACKENDS"
fi

# ---------------------------------------------------------------------------
# Build, retain, strip
# ---------------------------------------------------------------------------

# rivet-mcpd joins the card for ADR-035 phase 1. It is a TRANSPORT for
# `rivet mcp` (PLAN.md §71) and belongs in /system/bin beside rivet-mdnsd;
# nothing starts it yet, so a card carrying it behaves exactly as before.
# rivet-launcher joins the card for ADR-050 §2.1. It is TIER 2: the image
# carries it and the DOCUMENT decides whether it runs, exactly as
# rivet-bluetoothd and rivet-mdnsd do. The supervisor's definition ships it
# DISABLED, so a headless card carrying it behaves as every card before today.
#
# **It was an application until this ADR** — cmd/rivet-launcher's own header
# records the 2026-09-01 ruling and deploy/launcher/Cargo.toml is the manifest
# that shipped it as one. That manifest and the deploy recipe in it still work
# and are the way to iterate on the launcher without building a card.
#
# rivet-proxyd joins the card for ADR-060: the machine's front door is part of
# the operating system. Tier 2 like the launcher -- the supervisor's definition
# ships it DISABLED and the document's role enables it -- and it runs under its
# own uid and Landlock ruleset rather than as root.
#
# rivet-dnsd joins the card for ADR-047: the name service, shipped DISABLED
# like mdnsd -- a document enables it where the fleet's names are served.
OURS="rivet-initd rivet-sysd rivet rivet-netd rivet-timed rivet-mdnsd rivet-deploy rivet-mcpd rivet-splash rivet-dhcp-hook rivet-bluetoothd rivet-launcher rivet-proxyd rivet-dnsd"

echo "building for $TARGET (release, symbols kept — ADR-005 §7.5)"
# shellcheck disable=SC2086
# **`--bin`, not `-p`.** `ADR-033` §4 split the three names Cargo distinguishes:
# the PACKAGE is `rivetos-initd`, the BINARY is `rivet-initd`, and `$OURS` is a
# list of binaries because that is what goes into the image and what this script
# strips and copies. Selecting by `-p` here would need a second list of package
# names to keep in step with this one, which is `ADR-011` in a shell script.
# **`--message-format` so cargo says where it put them.** Reading this from a
# path this script composes is what shipped `PLAN.md` 88: the compile honoured
# CARGO_TARGET_DIR and the staging did not, so the card was built from binaries
# one commit old and nothing failed. Cargo names each artifact it produced;
# that is the authority, and no directory is guessed at.
#
# `json-render-diagnostics` keeps warnings and errors human-readable on stderr
# while stdout carries the machine-readable half.
ARTIFACTS=$(mktemp)
trap 'rm -f "$ARTIFACTS"' EXIT
# shellcheck disable=SC2086
phase "cargo build (Rivet's own binaries)"
cargo build --release --target "$TARGET" --message-format=json-render-diagnostics \
    $(for c in $OURS; do printf -- '--bin %s ' "$c"; done) >"$ARTIFACTS"

# Every executable cargo reported, one per line. A binary that is already up
# to date is still reported, so this does not confuse "not rebuilt" with
# "not built" -- which is what an mtime check does, and why there is not one
# here: only rivet-mcpd relinked on the build that found this bug, and the
# other nine were correctly untouched since the previous commit.
EXECUTABLES=$(sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' "$ARTIFACTS")

# --- the compositor, which is a SECOND cargo invocation and has to be --------
#
# **`cmd/rivet-displayd` is its own workspace** — the repository root's
# `Cargo.toml` argues it under `exclude`, and the short version is that it links
# Mesa, Mesa stages no static `libgbm`, and `-C target-feature=-crt-static` can
# only be set in a `.cargo/config.toml` that cargo actually reads.
#
# **Which is why this is a subshell that changes directory**, and the one place
# in this script that does. Cargo reads `.cargo/config.toml` from the
# INVOCATION directory upward, so `cargo build --manifest-path cmd/rivet-displayd/…`
# run from the repository root would read the ROOT's config, link statically,
# and fail on `-lgbm` — with an error about a library rather than about the
# working directory, which is the part that would cost an afternoon.
#
# It is not in `$OURS` for the same reason: that list is passed as `--bin` to
# the one workspace build above.
#
# **`$DISPLAYD_BIN` is how it reaches the staging loop**, and it is a separate
# variable rather than an entry in `$OURS` because those two lists are used for
# different things: `$OURS` is passed to `cargo build` as `--bin`, and this
# binary comes from a different `cargo build` entirely.
#
# It was empty on the first card built with this block, 2026-09-20, and the
# card came out with no compositor on it while every check passed — the staging
# loop walked `$OURS` alone, so nothing was in `$STAGED`, so `must_be_runnable`
# returned early on a missing file and `DISPLAYD_ARG` stayed unset. Three
# guards in a row, each correct in isolation, each silent. That is this file's
# own recurring shape — a build that SUCCEEDS AND IS WRONG — and the gate at
# the bottom of this script is what caught it, by listing what is on the image
# rather than what the build meant to put there.
DISPLAYD_BIN=""
if [ -d "$REPO/cmd/rivet-displayd" ]; then
    phase "cargo build (rivet-displayd, its own workspace — ADR-058 §5.1)"
    DISPLAYD_ARTIFACTS=$(mktemp)
    (
        cd "$REPO/cmd/rivet-displayd" || exit 1
        cargo build --release --target "$TARGET" \
            --message-format=json-render-diagnostics
    ) >"$DISPLAYD_ARTIFACTS"
    EXECUTABLES="$EXECUTABLES
$(sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' "$DISPLAYD_ARTIFACTS")"
    rm -f "$DISPLAYD_ARTIFACTS"
    DISPLAYD_BIN=rivet-displayd
fi

binary_path() {
    printf '%s\n' "$EXECUTABLES" | while IFS= read -r exe; do
        [ "$(basename "$exe")" = "$1" ] && printf '%s' "$exe" && break
    done
}

UNSTRIPPED="$REPO/out/debug/$DESCRIBE"
phase "staging and stripping"
STAGED=$(mktemp -d)
trap 'rm -rf "$STAGED"; rm -f "$ARTIFACTS"' EXIT

# Resolved before anything is copied, so a missing one stops the build here
# rather than after 640 MB of image has been written.
for b in $OURS; do
    path=$(binary_path "$b")
    if [ -z "$path" ] || [ ! -f "$path" ]; then
        echo "tools/card/build.sh: cargo did not report an executable for $b" >&2
        echo "" >&2
        echo "  It was passed as --bin, so either the build did not run or the name" >&2
        echo "  in \$OURS is not a binary this workspace produces (ADR-033 4 --" >&2
        echo "  the PACKAGE and the BINARY are different names)." >&2
        exit 1
    fi
done
echo "staging from $(dirname "$(binary_path rivet-initd)")"

mkdir -p "$UNSTRIPPED"
echo
printf '%-22s %12s %12s\n' BINARY "WITH SYMBOLS" STRIPPED
for b in $OURS $DISPLAYD_BIN; do
    src=$(binary_path "$b")
    cp "$src" "$UNSTRIPPED/$b"
    cp "$src" "$STAGED/$b"
    "$STRIP" "$STAGED/$b"
    printf '%-22s %12s %12s\n' "$b" \
        "$(stat -c %s "$UNSTRIPPED/$b")" "$(stat -c %s "$STAGED/$b")"
done

# --- every binary on this card is checked before the image is assembled -------
#
# **`PLAN.md` §123, Wave 4.1.** This card carries no dynamic loader: `ADR-003`
# rules static musl, `ADR-005` §7 composes a userland out of static binaries,
# and there is no `ld-musl-aarch64.so.1` anywhere on the image. A dynamically
# linked binary on it does not fail at build time and does not fail at install
# time — it fails at `execve`, on the board, with `ENOENT` against an
# interpreter that is not there, reported as if the program were missing.
#
# **This was not hypothetical, which is why the gate is here.**
# `tools/buildroot/configs/rivet_aarch64_defconfig` set `BR2_STATIC_LIBS=y`
# until Mesa was added; Mesa carries `depends on !BR2_STATIC_LIBS`, so the
# configuration moved to `BR2_SHARED_STATIC_LIBS=y` and its own comment says the
# first build after that "is audited rather than shipped". Nothing performed
# that audit and nothing checked afterwards, so whether `git`,
# `git-http-backend`, `wpa_supplicant`, `wpa_cli` and `busybox` are still static
# has been an open question in every card built since. The audit of 2026-09-03
# named it as the project's own recurring failure shape: a build that **succeeds
# and is wrong**.
#
# **The open question is now answered, and the answer is good.** Run against the
# 2026.02.3 tree on 2026-09-03, with the cross `readelf` this gate uses:
#
#   /br/target/usr/bin/git                            static
#   /br/target/usr/libexec/git-core/git-http-backend  static
#   /br/target/usr/sbin/wpa_supplicant                static
#   /br/target/usr/sbin/wpa_cli                       static
#   /br/target/bin/busybox                            static
#
# So no card already written carries a binary it cannot run. That is a
# MEASUREMENT of one tree at one moment and not a property of the
# configuration: `BR2_SHARED_STATIC_LIBS=y` means Buildroot builds both kinds
# and each package chooses, so the next package added — or the next version of
# one of these — can change any of those five lines without anything saying so.
# Which is the argument for the gate rather than against it.
#
# The gate runs every time, for free.
READELF="$BR_OUTPUT/host/bin/aarch64-linux-readelf"
[ -x "$READELF" ] || {
    echo "no cross readelf at $READELF, so nothing can check what is on this card" >&2
    echo "  It comes from the same Buildroot tree as \$STRIP, which was found." >&2
    exit 1
}

must_be_static() {
    _what=$1
    [ -f "$_what" ] || return 0
    if "$READELF" -lW "$_what" 2>/dev/null | grep -qi 'program interpreter'; then
        echo >&2
        echo "REFUSED: $_what is DYNAMICALLY LINKED and this card has no loader." >&2
        echo "  It would install cleanly and fail at exec, on the board, reported" >&2
        echo "  against a program that is present. What it wants:" >&2
        "$READELF" -dW "$_what" 2>/dev/null | grep NEEDED >&2
        echo "  See tools/buildroot/configs/rivet_aarch64_defconfig — Mesa forced" >&2
        echo "  BR2_SHARED_STATIC_LIBS=y and this is what that costs." >&2
        exit 1
    fi
    "$READELF" -hW "$_what" 2>/dev/null | grep -q 'Machine: *AArch64' || {
        echo "REFUSED: $_what is not an AArch64 binary" >&2
        "$READELF" -hW "$_what" 2>/dev/null | grep Machine >&2
        exit 1
    }
}

# **A VENDORED binary may be dynamic, if every library it asks for is on the
# card.** Rivet's own binaries are still held to `must_be_static` above: those
# are cargo's, built against `aarch64-unknown-linux-musl`, and a dynamic one
# would mean the build did something nobody asked for.
#
# Until 2026-09-20 this gate demanded static of these too, on the strength of a
# measurement taken on 2026-09-03 — and by the next time the tree was built from
# scratch, `git` had flipped to shared and wanted `libz.so.1`. Refusing was
# right; refusing *for ever* was not, because `BR2_SHARED_STATIC_LIBS=y` makes
# that flip a normal event rather than a fault, and the card has shipped a
# loader and a library closure since Mesa arrived.
#
# What is checked is now the thing that actually matters: not "is this static"
# — a snapshot that expires — but "can this exec on the card we are writing",
# which is a property of this image and is true or false right now.
must_be_runnable() {
    _what=$1
    [ -f "$_what" ] || return 0
    if "$READELF" -lW "$_what" 2>/dev/null | grep -qi 'program interpreter'; then
        [ -n "$LOADER_ARG" ] || {
            echo >&2
            echo "REFUSED: $_what is DYNAMICALLY LINKED and this card has no loader." >&2
            echo "  Nothing dynamic can exec here. The loader comes from the cross" >&2
            echo "  sysroot; see \$LOADER above." >&2
            exit 1
        }
        for _dep in $("$READELF" -dW "$_what" 2>/dev/null \
            | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
            # musl's libc IS the loader, already installed.
            [ "$_dep" = "libc.so" ] && continue
            case " $SHIPPED " in
                *" $_dep "*) ;;
                *)
                    echo >&2
                    echo "REFUSED: $_what needs $_dep and this card does not ship it." >&2
                    echo "  It would install cleanly and fail at exec, on the board," >&2
                    echo "  reported against a program that is present." >&2
                    echo "  \`collect_libraries\` seeds its closure from these same" >&2
                    echo "  binaries, so a library missing HERE was missing from every" >&2
                    echo "  search directory — not merely unlisted." >&2
                    exit 1
                    ;;
            esac
        done
        echo "    $(basename "$_what"): dynamic, and every library it needs is on this card"
    fi
    "$READELF" -hW "$_what" 2>/dev/null | grep -q 'Machine: *AArch64' || {
        echo "REFUSED: $_what is not an AArch64 binary" >&2
        "$READELF" -hW "$_what" 2>/dev/null | grep Machine >&2
        exit 1
    }
}

echo
echo "--- every binary on this card: AArch64, and able to exec ---"
for b in $OURS; do
    must_be_static "$STAGED/$b"
done
# **The one of ours that is NOT held to `must_be_static`** — `ADR-058` §5.1.
# It links Mesa and Mesa stages no static `libgbm`, so it is dynamic on
# purpose. It is held to the question that actually matters instead: can this
# exec on the card being written. If `libxkbcommon.so.0` is not in `$SHIPPED`,
# this is where the build stops — before an image is assembled, rather than on
# a board with no shell to diagnose it.
#
# **The `-f` test is the fix for a card that shipped without it.**
# `must_be_runnable` returns quietly for a path that is not there, which is
# right for the vendored binaries — an absent `wpa_supplicant` is a card
# without wireless — and exactly wrong here, because this one was BUILT and the
# build said so. The check is therefore on whether it was built, not on whether
# it is present: the two disagreeing is the bug.
if [ -n "$DISPLAYD_BIN" ]; then
    [ -f "$STAGED/rivet-displayd" ] || {
        echo "REFUSED: rivet-displayd was built and is not in the staging directory." >&2
        echo "  The staging loop walks \$OURS and \$DISPLAYD_BIN; one of them lost it." >&2
        exit 1
    }
    must_be_runnable "$STAGED/rivet-displayd"
fi
# The vendored ones, which are the reason this gate exists. Unquoted where the
# variable may be empty — an absent `wpa_supplicant` is a card without wireless
# and not an error, and `must_be_runnable` returns for a path that is not there.
# These are held to "can exec on this card" rather than "is static"; the
# function says why.
for f in "$GIT" "$HTTP_BACKEND" "$SUPPLICANT" "$WPA_CLI" "$UDHCPC" "$NU"; do
    must_be_runnable "$f"
done
echo "--- checked $(printf '%s ' $OURS | wc -w) of ours and 6 vendored ---"

# `nu` and the vendored C arrive already stripped — they are inputs this workspace
# does not build (ADR-009 §6), and their own recipes strip them. Copied rather
# than stripped again so the bytes on the card are the bytes those recipes
# produced and measured.
# femto is NOT packed - `ADR-005` §5.5.2, enacted 2026-08-30. §5.5.2.8 ruled it
# revived and §5.5.2.9 corrected WHERE: it is a PANE IN `rivet-fleet`, the
# maintainer's own editor reaching a file over the control plane, not a binary
# on the card. `system::FEMTO_PATH` already said a card with no editor is a
# supported card, which is why this is one line rather than a redesign.
#
# **The crate stays.** Wiring it into `rivet-fleet` is the other half of the
# same ruling and is still to do (`PLAN.md` §102.4.1 item 13).
# rivet-splash is one of OURS, so it is already built and stripped into
# $STAGED. Passed as an option rather than a positional for the reason
# `system::SPLASH_PATH` states: a card with no splash boots identically and
# shows the console, which is every card built before 2026-08-30.
SPLASH_ARG="--splash $STAGED/rivet-splash"

# rivet-bluetoothd is one of OURS on the same terms, and it is an OPTION for a
# different reason than the splash is -- `system::BLUETOOTHD_PATH` states it:
# the binary is paired with the KERNEL, not with a screen. It needs CONFIG_BT,
# which arrived in tools/buildroot/linux/rivet.config on 2026-09-05 and is in no
# kernel built before that. A card assembled here takes the kernel from the same
# Buildroot tree, so passing it unconditionally is right HERE and would not be
# right in cmd/rivet-image, which is handed a kernel it did not build.
#
# The service ships DISABLED (cmd/rivet-sysd's definition), so this puts the
# program on the card and holds no radio until the document says so.
BLUETOOTHD_ARG="--bluetoothd $STAGED/rivet-bluetoothd"

# rivet-launcher is one of OURS on BLUETOOTHD_ARG's terms exactly -- ADR-050
# 2.1's tier 2. The service ships DISABLED (cmd/rivet-sysd's definition), so
# this puts the program on the card and holds no screen until the document says
# so, which is what "headless leaves launcher off, kiosk adds launcher" means.
LAUNCHER_ARG="--launcher $STAGED/rivet-launcher"

# rivet-proxyd is one of OURS on LAUNCHER_ARG's terms -- ADR-060. Passed on
# every card, and the listing gate below refuses a card that lacks it: a
# server or member whose image has no proxy has no front door, and nothing
# can carry one any more.
PROXYD_ARG="--proxyd $STAGED/rivet-proxyd"

# rivet-dnsd is one of OURS on PROXYD_ARG's terms -- ADR-047. The service
# ships DISABLED, so every card carries it and none runs it until its document
# says so.
DNSD_ARG="--dnsd $STAGED/rivet-dnsd"

# rivet-displayd joins the card for ADR-058 §4.1, on the launcher's terms
# exactly: tier 2, the supervisor's definition names this path and ships it
# DISABLED, so a headless card carrying it behaves as one without it.
#
# Conditional where the launcher's is not, because this crate is outside the
# workspace and a tree that has not built it yet would otherwise stop the card
# build on a missing file.
DISPLAYD_ARG=""
[ -n "$DISPLAYD_BIN" ] && DISPLAYD_ARG="--displayd $STAGED/rivet-displayd"

# **Paired with the client deliberately.** cmd/rivet-image REFUSES a card that
# carries udhcpc without the program it execs: udhcpc would run, take an offer,
# and exec a path that is not there -- which looks exactly like a network that
# is simply down. So this is passed if and only if the client is.
#
# It is composed HERE rather than beside UDHCPC_ARG above because
# rivet-dhcp-hook is one of $OURS: it is built and stripped into $STAGED, and
# $STAGED does not exist until a few lines above this one. Composing it earlier
# expanded to `--dhcp-hook /rivet-dhcp-hook`, which is a real path and the wrong
# one.
DHCP_HOOK_ARG=""
if [ -n "$UDHCPC_ARG" ]; then DHCP_HOOK_ARG="--dhcp-hook $STAGED/rivet-dhcp-hook"; fi

FEMTO_ARG=""

[ -f "$NU" ] && NU_ARG="--nushell $NU" || NU_ARG=""

# **The trust roots.** Taken from the build container, which is where a bundle
# that is kept up to date already exists -- rebuilding Buildroot to add
# ca-certificates is hours of work for the same bytes. Its digest and count are
# printed, because "which certificate authorities does this image trust" is a
# question an operator is entitled to answer without unpacking the card.
#
# Absent, the image ships none and every TLS verification on the machine fails
# with `certificate signed by unknown authority`. That is a WARNING and not a
# refusal, on the app-key rule: a card that cannot verify TLS still boots, still
# takes an update and can still be repaired.
# **The system typeface.** Committed to this repository rather than fetched --
# assets/fonts/README.md carries why -- so the path is inside the tree and a
# build needs nothing from anybody's home directory. PLAN.md 104.38.
FONT_SRC=${RIVET_SYSTEM_FONT:-$REPO/assets/fonts/Geist.ttf}
MONO_SRC=${RIVET_SYSTEM_MONO:-$REPO/assets/fonts/GeistMono.ttf}
ICON_SRC=${RIVET_ICON_FONT:-$REPO/assets/fonts/MaterialIcons.ttf}
if [ -f "$FONT_SRC" ]; then
    FONT_ARG="--font $FONT_SRC"
    echo "--- system font: $(wc -c < "$FONT_SRC") bytes from ${FONT_SRC#"$REPO"/}"
else
    FONT_ARG=""
    echo "--- NO SYSTEM FONT at $FONT_SRC: every screen this image draws will be wordless." >&2
fi
# **The monospace** -- added 2026-09-04. Geist Mono, the same family and the
# same OFL.txt as the sans above.
#
# Absent is a WARNING and not a refusal, on the sans's rule one step down: a
# screen with no monospace still draws every word, and the number that changes
# in place shifts sideways as it changes. That is legible and looks broken.
if [ -f "$MONO_SRC" ]; then
    MONO_ARG="--mono-font $MONO_SRC"
    echo "--- system mono: $(wc -c < "$MONO_SRC") bytes from ${MONO_SRC#"$REPO"/}"
else
    MONO_ARG=""
    echo "--- no system mono at $MONO_SRC: changing numbers will shift as they change." >&2
fi
if [ -f "$ICON_SRC" ]; then
    ICON_ARG="--icon-font $ICON_SRC"
    echo "--- icon font: $(wc -c < "$ICON_SRC") bytes from ${ICON_SRC#"$REPO"/}"
else
    ICON_ARG=""
    echo "--- no icon font at $ICON_SRC: screens will have words and no symbols." >&2
fi

# **The zone database** — ADR-021 §9, 2026-09-02.
#
# Taken from the build container, for the CA bundle's reason above and with the
# same reasoning applying more strongly: tzdata is ARCHITECTURE-INDEPENDENT
# data. A tzfile is big-endian by RFC 8536 whoever compiled it, so the bytes
# Buildroot's `zic` would produce are the bytes already sitting in this
# container — and BR2_PACKAGE_TZDATA would be hours of Buildroot for them.
#
# **The container's copy is also the FRESHER one.** tzdata is republished
# several times a year when a jurisdiction changes its rules, and this
# container's is whatever its base image last shipped; a Buildroot package
# would be pinned to whatever version that Buildroot release pinned.
#
# `cmd/rivet-image` reads every file and ships only what actually begins
# `TZif`, and refuses the `posix/` and `right/` subtrees by name — see
# `zone_database` in `cmd/rivet-image/src/main.rs` for what each exclusion
# costs and why. So this is a directory and not a list.
#
# Absent, the image ships no database and a document naming a zone gets no
# /etc/localtime — the machine keeps UTC and the supervisor says so on every
# boot. That is a WARNING and not a refusal, on the CA bundle's rule: a card
# whose clock is UTC still boots, still takes an update and can be repaired.
ZONEINFO_SRC=${RIVET_ZONEINFO:-/usr/share/zoneinfo}
if [ -d "$ZONEINFO_SRC" ]; then
    ZONEINFO_ARG="--zoneinfo $ZONEINFO_SRC"
    echo "--- zone database: $(find "$ZONEINFO_SRC" -path "$ZONEINFO_SRC/posix" -prune -o -path "$ZONEINFO_SRC/right" -prune -o -type f -print | wc -l) files offered from $ZONEINFO_SRC (rivet-image ships the ones that are tzfiles)"
else
    ZONEINFO_ARG=""
    echo "--- NO ZONE DATABASE at $ZONEINFO_SRC: this card can only keep UTC, and a" >&2
    echo "    document naming a timezone will get no /etc/localtime. Set RIVET_ZONEINFO." >&2
fi

CA_SRC=${RIVET_CA_BUNDLE:-/etc/ssl/certs/ca-certificates.crt}
if [ -f "$CA_SRC" ]; then
    CA_ARG="--ca-bundle $CA_SRC"
    echo "--- CA bundle: $(grep -c 'BEGIN CERTIFICATE' "$CA_SRC") certificates, $(wc -c < "$CA_SRC") bytes, sha256 $(sha256sum "$CA_SRC" | cut -c1-16)... from $CA_SRC"
else
    CA_ARG=""
    echo "--- NO CA BUNDLE at $CA_SRC: this image will trust no certificate authority," >&2
    echo "    so every HTTPS request on the machine fails to verify. Set RIVET_CA_BUNDLE." >&2
fi
[ -f "$NU" ] || echo "note: no nu at $NU — the card's login shell will be cmd/rivet"

DEBUG_ARGS=""
if [ "$DEBUG" = yes ]; then
    GDBSERVER="$BR_OUTPUT/target/usr/bin/gdbserver"
    STRACE_BIN="$BR_OUTPUT/target/usr/bin/strace"
    for f in "$GDBSERVER" "$STRACE_BIN"; do
        [ -f "$f" ] || { echo "--debug needs $f — run tools/debug/build.sh" >&2; exit 1; }
    done
    DEBUG_ARGS="--gdbserver $GDBSERVER --strace $STRACE_BIN"
    if [ -z "$NU_ARG" ]; then
        # ADR-005 §7.3: cmd/rivet refuses to exec anything but itself, so a
        # card with a debugger and no shell cannot start one. Refused rather
        # than built, because the card would look right and be useless.
        echo "--debug without a nu: nothing on that card could START a debugger" >&2
        echo "(ADR-005 §7.3 — cmd/rivet refuses to exec anything but itself)" >&2
        exit 1
    fi
fi

OUT="out/rivet-$VERSION-$DESCRIBE${DEBUG_ARGS:+-debug}-aarch64-rpi4-card.img"
mkdir -p out

# --- what this card will trust (ADR-028 section 5.5.5) ------------------------
#
# `cmd/rivet-image` refuses to build an image with an empty trust store unless
# it is told to, because a card that trusts nobody cannot verify an update and
# therefore cannot be updated safely -- and that is a thing to discover here
# rather than on a board.
#
# **The keys come from keys/**, as public halves: keys/rivet-system.pub for
# `update`, `extension` and `card`, keys/rivet-app.pub for applications. A
# clone holds none until its builder adds their own -- `cargo rivet setup`
# mints the seeds and writes each public half beside its seed -- and a build
# with no system key is refused below. A private seed never enters the
# repository; it stays on the machine that signs, which is ADR-015 section 4's
# first bullet.
#
# So a card built here trusts the system key for `update`, `extension` and
# `card` by default. That is what ADR-028 section 5.5.6 means by an image
# saying what it trusts, and it is the difference between a board that can take
# an update and one that cannot. `card` is there so a fleet server's repository
# store can hold the cards it would restore a host from (ADR-063 section 6): the
# key already signs system updates, which are strictly more powerful than a
# card, and a machine's own store refuses a card by kind whoever signed it.
#
# ADR-028 section 5.6's ROOT key is still deliberately unbuilt: a root buys an
# offline anchor, and the offline discipline IS the feature, so it should be
# generated on the day it will actually be kept offline rather than sit on the
# same machine as the key it delegates to.
#
# Set RIVET_TRUSTED_KEY to trust a different key, or RIVET_NO_TRUST=1 to
# build a card that trusts nothing on purpose.
# ---------------------------------------------------------------------------
# The provisioning document
# ---------------------------------------------------------------------------
#
# Without one, the card carries rivet-image's template: a schema version and
# nothing else. That card BOOTS AND NOBODY CAN REACH IT -- with no [network]
# table it takes no address, so the control plane has nothing to listen on.
#
# Editing rivet.toml on p1 after burning works -- it is FAT and mounts on any
# workstation -- but it is a step performed at the wrong end of the day, and a
# card you cannot log into is not a card you can test.
#
# DHCP IS BUILT. rivet-config parses and validates `method = "dhcp"` no
# differently from `static` (rivet_config::network); the supervisor runs BusyBox
# udhcpc as a service per DHCP interface and rivet-netd applies the lease it
# records (ADR-004 §8), and this script puts that client on the card whenever
# Buildroot's busybox binary is present -- see UDHCPC_ARG above. A static
# address is still the shape that is reachable at a known address the moment
# the card boots; tools/card/provisioning.example.toml is that shape.
DOCUMENT_ARG=""
IDENTITY_ARG=""
if [ -n "${RIVET_IDENTITY:-}" ]; then
    # Checked here as well as in cmd/rivet-image, because the message a build
    # gives is the one an operator reads first.
    for f in device.seed device.crt peers; do
        if [ ! -f "$RIVET_IDENTITY/$f" ]; then
            echo "tools/card/build.sh: RIVET_IDENTITY names $RIVET_IDENTITY and $f is not in it." >&2
            echo "  \`cargo rivet setup\` writes all three. \`cargo rivet identity new" >&2
            echo "  --directory DIR\` writes device.seed and device.crt on their own;" >&2
            echo "  \`peers\` is one hex public key per line and is yours." >&2
            exit 1
        fi
    done
    # **An EMPTY peers file is a card nobody can reach**, and it looks exactly
    # like a good one to the loop above. Added 2026-09-05: `cargo rivet
    # provision` refuses a card whose peers file does not name the operator, and
    # this is the same check at the end that can still do something about it.
    if ! grep -qE '^[[:space:]]*[0-9a-fA-F]{64}[[:space:]]*$' "$RIVET_IDENTITY/peers"; then
        echo "tools/card/build.sh: $RIVET_IDENTITY/peers holds no 64-character hex key." >&2
        echo "  A card built from it brings up its control plane and authenticates" >&2
        echo "  nobody -- on hardware with no shell, so the only repair is another card." >&2
        echo "  \`cargo rivet identity show --directory ~/.rivet/identity/operator\`" >&2
        echo "  prints the line that belongs in it." >&2
        exit 1
    fi
    IDENTITY_ARG="--identity $RIVET_IDENTITY"
    # Said on success as well: the README tells an operator to read this line
    # to learn whether anything will be able to reach the machine, and a
    # silence is not an answer.
    echo "--- identity: carried; peers names $(grep -cE '^[[:space:]]*[0-9a-fA-F]{64}[[:space:]]*$' "$RIVET_IDENTITY/peers") key(s) that may reach this card"
else
    # **Said out loud, 2026-09-05.** This branch used to be silent, and it is
    # the one that produces the failure the whole first-run story is about: an
    # unset variable, a card that builds perfectly, and a board that comes up
    # with no control plane at all. The RIVET_DOCUMENT branch below has said its
    # equivalent since the file was written; this one had nothing.
    #
    # A WARNING and not a refusal, because a card with no identity is a real
    # thing to build -- the QEMU harness takes one, and so does anybody testing
    # the boot path. The refusal that stops a person WRITING one to a card lives
    # in `cargo rivet provision`, which is the last moment it can be caught.
    echo "--- identity: NONE. rivet-mcpd will not start, so NOTHING CAN REACH THIS CARD."
    echo "    Set RIVET_IDENTITY=<dir> -- \`cargo rivet setup\` writes one at"
    echo "    ~/.rivet/identity/board, with a peers file naming your operator key."
fi
if [ -n "${RIVET_DOCUMENT:-}" ]; then
    [ -f "$RIVET_DOCUMENT" ] || {
        echo "build.sh: RIVET_DOCUMENT=$RIVET_DOCUMENT does not exist" >&2
        exit 1
    }
    DOCUMENT_ARG="--document $RIVET_DOCUMENT"
    echo "--- provisioning: $RIVET_DOCUMENT"
    # Named rather than dumped: a real document carries an address, and
    # AGENTS.md section 8.3 keeps it out of anything that might be captured
    # into the record.
    grep -qE '^[[:space:]]*\[network\.' "$RIVET_DOCUMENT" || echo "    NOTE: no [network] table -- this card will take no address" >&2
else
    echo "--- provisioning: NONE. This card boots and nothing can reach it."
    echo "    Set RIVET_DOCUMENT=<path> -- see tools/card/provisioning.example.toml"
fi

# **The staging bundle** -- ADR-049, the material this machine's APPLICATIONS
# are handed before they first start: a fleet server's certificates, its
# operator credential and its one-record register.
#
# rivet-provision writes it beside the document, and cmd/rivet-image PARSES it
# before it writes a partition, so a bundle that will not read is refused here
# rather than on a headless board.
#
# Silent when unset and deliberately so, unlike the two above: most cards stage
# nothing, and a warning on every build is a warning nobody reads. What makes
# its absence visible is rivet-provision, which prints the whole command line
# including this variable when it has written one.
STAGING_ARG=""
if [ -n "${RIVET_STAGING:-}" ]; then
    [ -f "$RIVET_STAGING" ] || {
        echo "build.sh: RIVET_STAGING=$RIVET_STAGING does not exist" >&2
        exit 1
    }
    STAGING_ARG="--staging $RIVET_STAGING"
    echo "--- staging: $RIVET_STAGING"
    # Counted rather than listed. The file holds private keys as hex, and
    # AGENTS.md section 8.3 keeps material like that out of anything that might
    # be captured into the record.
    echo "    $(grep -cE '^[a-zA-Z0-9]' "$RIVET_STAGING") line(s), of which one is the version"
fi

# **The applications this card CARRIES** -- ADR-050 4, and the thing that makes
# SPEC-05 1.1 literally true: `fleetd` comes up running because the card brought
# it, not because somebody deployed it afterwards.
#
# RIVET_APPLICATIONS is a space-separated list of signed .rvt paths. Each rides
# on the BOOT partition; PID 1 lands it in /state/carried on the first boot and
# the supervisor asks rivet-deploy to install it -- the same route
# `cargo rivet deploy` takes, which is ADR-050 4's whole argument for carrying
# payloads rather than baking the programs into the image.
#
# cmd/rivet-image VERIFIES each one against the trust store THIS image is about
# to carry, so a payload signed by a key this card would refuse is refused here
# rather than on a headless board at eleven at night. The name it lands under
# comes from the payload's SIGNED header, never from the file you named.
APPLICATION_ARGS=""
APPLICATION_COUNT=0
for _app in ${RIVET_APPLICATIONS:-}; do
    [ -f "$_app" ] || {
        echo "build.sh: RIVET_APPLICATIONS names $_app and there is no file at that path" >&2
        echo "    cargo rivet package writes one; the list is space-separated." >&2
        exit 1
    }
    APPLICATION_ARGS="$APPLICATION_ARGS --application $_app"
    APPLICATION_COUNT=$((APPLICATION_COUNT + 1))
done

# **The fleet repository this card resolves its ROLE's payloads from** --
# ADR-050 §5.1, PLAN-Feature-Set-2026-09-12 §4. Where RIVET_APPLICATIONS names
# a payload by hand, this names a DIRECTORY -- the one `cargo rivet fleet
# publish` files objects under -- and cmd/rivet-image reads the composed
# RIVET_DOCUMENT's own enabled application services
# (rivet_config::Document::carried_services) to decide which products a
# server, member or kiosk role actually needs, resolving each by digest. The
# two are additive: either naming a product the other already resolved is the
# existing two-payloads-one-product refusal.
#
# **Refused without a document, on cmd/rivet-image's own rule**: there is no
# composed document to read a role's required services off. Checked here as
# well as there, on IDENTITY_ARG's footing above -- the message a BUILD gives
# is the one an operator reads first.
FLEET_REPOSITORY_ARG=""
if [ -n "${RIVET_FLEET_REPOSITORY:-}" ]; then
    if [ -z "${RIVET_DOCUMENT:-}" ]; then
        echo "build.sh: RIVET_FLEET_REPOSITORY is set and RIVET_DOCUMENT is not." >&2
        echo "  --fleet-repository resolves a ROLE's required payloads out of the composed" >&2
        echo "  document (ADR-050 §5.1), so there is nothing for it to read without one." >&2
        echo "  Set RIVET_DOCUMENT, or drop RIVET_FLEET_REPOSITORY and carry payloads by" >&2
        echo "  hand with RIVET_APPLICATIONS instead." >&2
        exit 1
    fi
    [ -d "$RIVET_FLEET_REPOSITORY" ] || {
        echo "build.sh: RIVET_FLEET_REPOSITORY=$RIVET_FLEET_REPOSITORY is not a directory" >&2
        exit 1
    }
    FLEET_REPOSITORY_ARG="--fleet-repository $RIVET_FLEET_REPOSITORY"
    echo "--- fleet repository: $RIVET_FLEET_REPOSITORY -- carrying this role's own payloads by digest"
else
    # **Said out loud, on IDENTITY_ARG's footing.** A role that enables an
    # application service (a fleet server's rivet-fleetd and rivet-proxyd, a
    # kiosk's auto-run application) gets NOTHING from this build unless it is
    # named by hand with RIVET_APPLICATIONS -- which is the gap SPEC-05 §1.1
    # exists to close, so its continued absence deserves a line every build
    # prints rather than one nobody thought to look for.
    echo "--- fleet repository: NONE. A role's own application services (ADR-050 §5.1)"
    echo "    carry nothing unless RIVET_APPLICATIONS names them by hand."
fi

TRUST_KEY="${RIVET_TRUSTED_KEY:-$REPO/keys/rivet-system.pub}"
# The APPLICATION key, added 2026-08-25. A SECOND key, and it must stay one.
#
# ADR-028 section 5.5: Rivet's own images and the applications you write are
# signed by different keys ON PURPOSE, so that a machine holding an application
# key cannot ship an operating system. cmd/rivet-image's own test asserts the
# system key is not entitled to sign applications, so widening the entry above
# is not the fix and would not compile past its test anyway.
#
# Until this existed, every card refused every application install -- correctly,
# and confusingly, at a signature-entitlement check nobody was looking at. That
# was measured on the board on 2026-08-25 (PLAN.md section 14) rather than
# predicted, because rivet-deploy said so in as many words.
APP_KEY="${RIVET_APP_KEY:-$REPO/keys/rivet-app.pub}"
if [ -n "${RIVET_NO_TRUST:-}" ]; then
    # **A card that trusts nobody cannot install what it carries** -- ADR-050
    # 4.3, at the moment it can still be fixed. cmd/rivet-image refuses this
    # too; the refusal is repeated here because the message a BUILD gives is the
    # one an operator reads first.
    if [ "$APPLICATION_COUNT" -gt 0 ]; then
        echo "build.sh: RIVET_APPLICATIONS asks this card to carry $APPLICATION_COUNT payload(s)" >&2
        echo "    and RIVET_NO_TRUST builds a card that trusts no signing key." >&2
        echo "    Every one of them would reach the machine and be refused there --" >&2
        echo "    on a headless board, in a journal, hours later. ADR-028 5.5.5." >&2
        exit 1
    fi
    TRUST_ARGS="--no-trust"
    echo "--- trust: NONE, by RIVET_NO_TRUST. This card can verify no update."
elif [ -f "$TRUST_KEY" ]; then
    TRUST_ARGS="--trusted-key rivet-system:update,extension,card:$TRUST_KEY"
    echo "--- trust: rivet-system (update, extension, card) from $TRUST_KEY"
    if [ -f "$APP_KEY" ]; then
        # **`library` joins `application` on 2026-08-31** -- Shared Rivets. One
        # key for both, because they are one trust domain: a library is a
        # payload an APPLICATION author publishes for their own applications.
        # ADR-028 §5.5 separates the SYSTEM key from the application one so that
        # the machine signing a hobby application cannot ship an operating
        # system, and that boundary is untouched -- the system key still may
        # sign neither of these.
        #
        # A card built before today refuses every library install at the
        # entitlement check, naming the key. That is the correct answer for that
        # card, and it is not a silent one.
        # apk joined the application key's mask 2026-09-27 -- ADR-028 5,
        # ADR-061 3: an APK is signed by the same key an application is, so
        # it rides in the mask rather than getting one of its own.
        TRUST_ARGS="$TRUST_ARGS --trusted-key rivet-app:application,library,apk:$APP_KEY"
        echo "--- trust: rivet-app (application, library, apk) from $APP_KEY"
    elif [ "$APPLICATION_COUNT" -gt 0 ]; then
        # **A REFUSAL when the card is asked to carry payloads** -- ADR-050 4.3,
        # and this is the failure that would otherwise be found at eleven at
        # night on the day a fleet is built. Without the application key in the
        # trust store, every carried payload is refused at the entitlement check
        # ON THE BOARD, so the card boots to a machine with no fleet server and
        # the account of why is a journal record on a headless box.
        #
        # cmd/rivet-image refuses this as well, naming the payload. The check is
        # repeated here because the message a BUILD gives is the one an operator
        # reads first, and because this one can name the key.
        echo "build.sh: RIVET_APPLICATIONS asks this card to carry $APPLICATION_COUNT payload(s)" >&2
        echo "    and there is no key at $APP_KEY, so this image would trust NO key" >&2
        echo "    entitled to sign an application. Every carried payload would be" >&2
        echo "    refused at the entitlement check on the BOARD (ADR-028 5.5)." >&2
        echo "" >&2
        echo "    Make one, or point RIVET_APP_KEY at the public half of the key" >&2
        echo "    those payloads were signed with:" >&2
        echo "      head -c 32 /dev/urandom > <seed>; cargo rivet key <seed> --out $APP_KEY" >&2
        exit 1
    else
        # A WARNING and not a refusal for a card that carries nothing, which is
        # the opposite of the system key above, and the asymmetry is deliberate:
        # a card that cannot verify an UPDATE can never be repaired over the
        # wire, while a card that cannot verify an APPLICATION is merely a card
        # that runs no applications -- recoverable by an update, which it can
        # still take.
        echo "    NOTE: $APP_KEY does not exist, so this card will REFUSE every" >&2
        echo "    application install at the entitlement check (ADR-028 5.5)." >&2
        echo "    Make one: head -c 32 /dev/urandom > <seed>; cargo rivet key <seed> --out $APP_KEY" >&2
    fi
else
    # Refused rather than quietly falling back to --no-trust. A card that cannot
    # verify an update is not discovered at build time; it is discovered on a
    # board, later, by somebody trying to fix something else (ADR-028 5.5.5).
    echo "build.sh: $TRUST_KEY does not exist, so this card would trust no key" >&2
    echo "    for its updates. A card is built against the keys of whoever signs" >&2
    echo "    them: \`cargo rivet setup\` mints yours into ~/.rivet/keys, and its" >&2
    echo "    rivet-system.pub and rivet-app.pub belong in keys/ (keys/README.md)." >&2
    echo "    Or point RIVET_TRUSTED_KEY at a public key, or set RIVET_NO_TRUST=1" >&2
    echo "    to build a development card on purpose. ADR-028 section 5.5.5." >&2
    exit 1
fi

echo
# shellcheck disable=SC2086
phase "cmd/rivet-image (partitions, filesystems, the 2 GiB write)"
# **Captured, not just streamed, so this script can count what was actually
# carried — fixed 2026-09-13.** `verify_carried_payloads` below used to trust
# only `$APPLICATION_COUNT` (RIVET_APPLICATIONS' own count), so a card whose
# payloads came ENTIRELY from `--fleet-repository` skipped verification
# outright: `[ "$APPLICATION_COUNT" -gt 0 ] || return 0` was true with nothing
# on the right of the `||` ever asked. `cmd/rivet-image`'s own report —
# `render_carried`, the "applications this card carries" banner — already
# names every payload it verified and is about to write, by BOTH sources at
# once, so this reads that report back rather than re-deriving the count.
IMAGE_REPORT=$(mktemp)
if ! cargo run --quiet -p rivetos-image -- "$OUT" \
    "$STAGED/rivet-initd" \
    "$STAGED/rivet-sysd" \
    "$STAGED/rivet" \
    "$STAGED/rivet-netd" \
    "$STAGED/rivet-timed" \
    "$STAGED/rivet-mdnsd" \
    "$GIT" \
    "$HTTP_BACKEND" \
    "$STAGED/rivet-deploy" \
    "$STAGED/rivet-mcpd" \
    --arch "$ARCH" \
    $KERNEL_ARGS $SPLASH_ARG $BLUETOOTHD_ARG $LAUNCHER_ARG $DISPLAYD_ARG $PROXYD_ARG $DNSD_ARG $APPLICATION_ARGS $SUPPLICANT_ARG $WPA_CLI_ARG $UDHCPC_ARG $DHCP_HOOK_ARG \
    $WIFI_FIRMWARE_ARG $LOADER_ARG $LIBRARY_ARGS \
    $SPLASH_ARG $FEMTO_ARG $NU_ARG $CA_ARG $ZONEINFO_ARG $FONT_ARG $MONO_ARG $ICON_ARG $DEBUG_ARGS $TRUST_ARGS $DOCUMENT_ARG $IDENTITY_ARG $STAGING_ARG $FLEET_REPOSITORY_ARG $PASSTHROUGH \
    >"$IMAGE_REPORT" 2>&1
then
    cat "$IMAGE_REPORT"
    rm -f "$IMAGE_REPORT"
    exit 1
fi
cat "$IMAGE_REPORT"
# One line per payload cmd/rivet-image actually verified and wrote, whichever
# of RIVET_APPLICATIONS or RIVET_FLEET_REPOSITORY it came from — the single
# authority `verify_carried_payloads` checks the boot partitions against.
TOTAL_CARRIED_COUNT=$(grep -c -- '-> app-.*\.rvt$' "$IMAGE_REPORT" || true)
rm -f "$IMAGE_REPORT"

# --- read the image back, and refuse if it is not what was asked for -------
#
# **`AGENTS.md` §5, applied to this script.** `cmd/rivet-image` returning zero
# is this build's CLAIM that a binary is on the card; the filesystem is the
# answer. Three times on 2026-08-28 they differed, and every time the build
# reported success:
#
#   1. `rivet-mcpd` added to `$OURS` — built, stripped, and installed nowhere,
#      because a name in that list reaches the BUILD and not the IMAGE.
#   2. Added to `cmd/rivet-image`'s path-validation list and not to the list
#      that places nodes.
#   3. `MCPD_ARG` defined here and never added to the invocation below. It is
#      a positional now -- ADR-035 phase 1 gave rivet-mcpd a supervisor
#      definition, so a card without the binary would be a card whose
#      supervisor names a path nothing put there.
#
# Each was invisible until a BOARD was asked what was in /system/bin, which is
# the most expensive possible place to find out. `PLAN.md` §72.5 is the general
# form: the operating system is more legible than the thing that builds it.
#
# `debugfs` reads an ext4 image without mounting it and without privilege — the
# same tool `tools/qemu/seed-card.sh` uses, and the same reason: no loop device,
# no root, no block device named (`AGENTS.md` §10.4).
phase "verifying the image"
verify_system_bin() {
    if ! command -v debugfs >/dev/null 2>&1; then
        echo "WARNING: debugfs is not on PATH, so this build DID NOT verify that the" >&2
        echo "         binaries it installed are on the image. That check is the only" >&2
        echo "         thing standing between a green build and a card missing a program" >&2
        echo "         (PLAN.md §72.3). Install e2fsprogs." >&2
        return 0
    fi

    _info=$(sgdisk -i 3 "$OUT")
    _start=$(printf '%s\n' "$_info" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
    _end=$(printf '%s\n' "$_info" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')
    _p3=$(mktemp)
    # **A 4 MiB block size, with the offsets in BYTES.** This was `bs=512`
    # until 2026-08-30, which made two million individual 512-byte reads to
    # copy a one-gigabyte partition -- and cost 124 seconds of a 154-second
    # build, which is to say 80% of it. The phase timer added the same day is
    # what found it; before that the whole build was one number and the guesses
    # about which part was slow were both wrong.
    #
    # `skip_bytes`/`count_bytes` keep the extraction sector-exact while letting
    # the block size be whatever is fast: without them a larger `bs` would need
    # the partition to start on a multiple of it, which is true of this layout
    # today and is not a thing to depend on.
    dd if="$OUT" of="$_p3" bs=4M iflag=skip_bytes,count_bytes         skip=$((_start * 512)) count=$(((_end - _start + 1) * 512)) status=none
    _listing=$(debugfs -R "ls -l /system/bin" "$_p3" 2>/dev/null)
    # **`/lib` is listed too since 2026-08-31**, and it is a second directory
    # rather than a wider glob because the loader is the first thing this build
    # installs OUTSIDE `/system/bin`. Everything above has been one directory's
    # worth of check since it was written, and a loader that silently failed to
    # land would produce a card on which every dynamic application refuses to
    # start with `ENOENT` from the kernel -- which is precisely the class of
    # silent absence this whole function exists for (PLAN.md §72.3).
    _lib_listing=$(debugfs -R "ls -l /lib" "$_p3" 2>/dev/null)
    # **`/lib/firmware/brcm` is listed too since 2026-09-05**, and it is here
    # because its absence had just cost a card.
    #
    # `R-144` enabled Buildroot's Bluetooth firmware package and taught the
    # kernel `CONFIG_BT_HCIUART_BCM`, and **nothing added the patchram to
    # `cmd/rivet-image`'s `FIRMWARE_FILES`** — a hard-coded list of exactly
    # three, all of them wireless. The build reported success and the image
    # carried a Bluetooth driver with no firmware to give it.
    #
    # That failure is invisible in every way that matters. `hci_bcm` asks for
    # the patchram at `hdev->setup`, so nothing happens until the daemon opens
    # the controller, and then the controller simply never appears — which
    # looks exactly like a radio that is not there. This function already
    # existed for that class of silent absence in `/system/bin` and `/lib`;
    # firmware was the third directory and nobody had extended it.
    _fw_listing=$(debugfs -R "ls -l /lib/firmware/brcm" "$_p3" 2>/dev/null)
    rm -f "$_p3"

    _missing=""
    for _want in $OURS; do
        printf '%s\n' "$_listing" | grep -q "[[:space:]]$_want\$" || _missing="$_missing $_want"
    done
    [ -f "$NU" ] && { printf '%s\n' "$_listing" | grep -q "[[:space:]]nu\$" || _missing="$_missing nu"; }
    # **The vendored binaries are checked too**, and this list is the reason:
    # $OURS is what this workspace BUILDS, so a file that comes from Buildroot
    # is invisible to the loop above. git-http-backend is the one that proved
    # it matters -- built on 2026-08-26, never copied, and nothing noticed
    # until a board was asked for `git http-backend` two days later.
    #
    for _want in git git-http-backend; do
        printf '%s\n' "$_listing" | grep -q "[[:space:]]$_want\$" || _missing="$_missing $_want"
    done
    # **Wireless is checked only when it was PASSED**, which is the difference
    # between it and the two above: those are required, these are a variant.
    # Checking unconditionally would fail every card built without a Buildroot
    # tree that has them; not checking at all is how rivet-splash reached a
    # deployed payload as an empty promise on 2026-08-30.
    if [ -n "$SUPPLICANT_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]wpa_supplicant\$" \
            || _missing="$_missing wpa_supplicant"
    fi
    if [ -n "$WPA_CLI_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]wpa_cli\$" \
            || _missing="$_missing wpa_cli"
    fi
    if [ -n "$UDHCPC_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]udhcpc\$" \
            || _missing="$_missing udhcpc"
    fi
    # The BLE observer, on the same terms and for the reason the comment above
    # gives about rivet-splash: an option that is passed and does not arrive is
    # an empty promise, and the supervisor's definition names the path.
    if [ -n "$BLUETOOTHD_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]rivet-bluetoothd\$" \
            || _missing="$_missing rivet-bluetoothd"
    fi
    # The kiosk menu, on the same terms -- ADR-050 2.1. It ships DISABLED, so
    # its silent absence would look exactly like a headless card and be found
    # only by somebody standing in front of a screen.
    if [ -n "$LAUNCHER_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]rivet-launcher\$" \
            || _missing="$_missing rivet-launcher"
    fi
    # The compositor, on the menu's terms exactly -- ADR-058 4.1. Its silent
    # absence is worse than the launcher's: the launcher would start, find no
    # socket, and report a failure against ITSELF.
    if [ -n "$DISPLAYD_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]rivet-displayd\$" \
            || _missing="$_missing rivet-displayd"
    fi
    # The front door -- ADR-060. It ships DISABLED, so its silent absence would
    # look like a machine whose role does not serve, and a server or member
    # would come up with nothing on 443 and no way left to carry one.
    if [ -n "$PROXYD_ARG" ]; then
        printf '%s\n' "$_listing" | grep -q "[[:space:]]rivet-proxyd\$" \
            || _missing="$_missing rivet-proxyd"
    fi
    # The loader, on wireless's terms: checked only when it was PASSED, because
    # a card without one is a supported card.
    if [ -n "$LOADER_ARG" ]; then
        printf '%s\n' "$_lib_listing" | grep -q "[[:space:]]ld-musl-aarch64.so.1\$" \
            || _missing="$_missing ld-musl-aarch64.so.1"
    fi

    if [ -n "$_missing" ]; then
        echo >&2
        echo "REFUSED: this build produced a card MISSING binaries it was asked to install:" >&2
        for _m in $_missing; do echo "    $_m" >&2; done
        echo >&2
        echo "    The build reported success and the image disagrees. A name reaches the" >&2
        echo "    card only if it is in \$OURS *and* passed to cmd/rivet-image *and* placed" >&2
        echo "    by that crate's system::tree. Check all three (PLAN.md §72.3)." >&2
        echo >&2
        echo "    The image is left in place for inspection: $OUT" >&2
        exit 1
    fi
    echo "--- verified on the image: $(printf '%s\n' "$_listing" | awk 'NF>1 {print $NF}' | grep -v '^\.' | tr '\n' ' ')"
    if [ -n "$LOADER_ARG" ]; then
        echo "--- verified on the image: /lib/$(printf '%s\n' "$_lib_listing" | awk 'NF>1 {print $NF}' | grep '^ld-musl')"
    fi

    # **The firmware, counted and named.** See `_fw_listing` above for what this
    # cost. A card is refused rather than shipped when the directory the build
    # was pointed at produced nothing, because a card with no firmware in it is
    # a machine with no wireless AND no Bluetooth, and both fail late and
    # quietly.
    if [ -n "$WIFI_FIRMWARE_ARG" ]; then
        _fw_names=$(printf '%s\n' "$_fw_listing" | awk 'NF>1 {print $NF}' | grep -v '^\.' || true)
        _fw_count=$(printf '%s\n' "$_fw_names" | grep -c . || true)
        if [ "$_fw_count" -eq 0 ]; then
            echo >&2
            echo "REFUSED: this build was given a firmware directory and the image has none." >&2
            echo >&2
            echo "    Nothing landed in /lib/firmware/brcm. The wireless half of the CYW43455" >&2
            echo "    will not associate and its Bluetooth half will never present a" >&2
            echo "    controller — and BOTH fail long after boot, so a card in this state" >&2
            echo "    looks like broken hardware rather than a broken build." >&2
            echo >&2
            echo "    A file reaches the card only if it is in cmd/rivet-image's" >&2
            echo "    system::FIRMWARE_FILES *and* present in the directory passed as" >&2
            echo "    --wifi-firmware. Check both." >&2
            echo >&2
            echo "    The image is left in place for inspection: $OUT" >&2
            exit 1
        fi
        echo "--- verified on the image: /lib/firmware/brcm — $_fw_count file(s): $(printf '%s\n' "$_fw_names" | tr '\n' ' ')"
    fi
}
verify_system_bin

# --- the carried payloads, read back off BOTH boot partitions ----------------
#
# **ADR-050 4, and the same argument verify_system_bin makes one partition
# over.** cmd/rivet-image returning zero is this build's CLAIM that a payload is
# on the card; the filesystem is the answer. A payload that was verified and
# then silently not written would produce a board that boots to no fleet server
# with nothing anywhere saying why -- which is precisely the class of failure
# PLAN.md 72.3 records for /system/bin.
#
# BOTH partitions, because that is what cmd/rivet-image writes and because an
# update swaps slots: a payload on p1 and not p2 is a card that loses its
# applications the first time it takes a system update.
#
# `mdir` reads a FAT partition without mounting it and without privilege, which
# is the same property debugfs has for ext4 (AGENTS.md 10.4). It is warned about
# rather than fatal when absent, on debugfs's terms.
#
# **`$TOTAL_CARRIED_COUNT`, not `$APPLICATION_COUNT` — fixed 2026-09-13.** The
# latter counts only RIVET_APPLICATIONS; a card whose payloads came entirely
# from RIVET_FLEET_REPOSITORY used to skip this whole function, because
# `[ "$APPLICATION_COUNT" -gt 0 ]` was false with nothing else ever asked.
# `$TOTAL_CARRIED_COUNT` is read from cmd/rivet-image's own report above and
# covers both sources — the same total the boot partitions must now hold.
verify_carried_payloads() {
    [ "$TOTAL_CARRIED_COUNT" -gt 0 ] || return 0
    if ! command -v mdir >/dev/null 2>&1; then
        echo "WARNING: mdir is not on PATH, so this build DID NOT verify that the" >&2
        echo "         $TOTAL_CARRIED_COUNT payload(s) it carries reached the boot partitions." >&2
        echo "         Install mtools." >&2
        return 0
    fi
    for _part in 1 2; do
        _info=$(sgdisk -i "$_part" "$OUT")
        _start=$(printf '%s\n' "$_info" | sed -n 's/^First sector: \([0-9]*\).*/\1/p')
        _end=$(printf '%s\n' "$_info" | sed -n 's/^Last sector: \([0-9]*\).*/\1/p')
        _p=$(mktemp)
        dd if="$OUT" of="$_p" bs=4M iflag=skip_bytes,count_bytes \
            skip=$((_start * 512)) count=$(((_end - _start + 1) * 512)) status=none
        _names=$(MTOOLS_SKIP_CHECK=1 mdir -i "$_p" -b ::/ 2>/dev/null | sed 's|^::/||')
        rm -f "$_p"
        _found=$(printf '%s\n' "$_names" | grep -c '^app-.*\.rvt$' || true)
        if [ "$_found" -ne "$TOTAL_CARRIED_COUNT" ]; then
            echo >&2
            echo "REFUSED: this build carried $TOTAL_CARRIED_COUNT payload(s) and boot partition" >&2
            echo "    p$_part holds $_found. The build reported success and the image disagrees." >&2
            echo "    A card written now boots to a machine missing an application, with" >&2
            echo "    nothing on it saying which (ADR-050 4)." >&2
            echo >&2
            echo "    The image is left in place for inspection: $OUT" >&2
            exit 1
        fi
        echo "--- verified on the image: p$_part carries $(printf '%s\n' "$_names" | grep '^app-.*\.rvt$' | tr '\n' ' ')"
    done
}
verify_carried_payloads

echo
report_phases

echo "--- the image ---"
ls -l "$OUT"
sha256sum "$OUT"
echo
echo "--- the symbols, kept ---"
echo "$UNSTRIPPED"
echo
echo "Point gdb at those, not at the card's copies. ADR-005 §7.5.2 item 3 is not"
echo "solved: nothing ties this directory to that image, so keep one of each in"
echo "play at a time or gdb will resolve the wrong build and not say so."
echo
echo "This is an image FILE. Writing a card is yours — check the device path"
echo "first (AGENTS.md §10.4)."

# --- retention: never leave more than RIVET_KEEP cards in out/ -------------
#
# 2026-08-26: `out/` had reached **36 GB** — seventeen 2 GiB card images and
# fourteen `out/debug/` symbol trees, accumulated one build at a time because
# nothing ever removed one. Sixteen of the images were FerriPi builds whose
# binaries no longer exist under any name.
#
# This is not a tidiness rule. §7.5.2 item 3 above says **keep one card and one
# $UNSTRIPPED in play at a time or gdb resolves the wrong build and does not say
# so** — and a directory holding fourteen of each is precisely the state that
# warning describes. The retention below is that warning, enforced.
#
# Newest-first by mtime, because the current card and the one being replaced are
# what an A/B rollback needs and nothing older has a use. The `out/debug/` tree
# for a build is removed WITH its image, so the pairing the warning depends on
# cannot drift.
KEEP=${RIVET_KEEP:-2}

if [ "$KEEP" -gt 0 ] 2>/dev/null; then
    # shellcheck disable=SC2012
    STALE=$(ls -t "$REPO"/out/*-card.img 2>/dev/null | tail -n +$((KEEP + 1)))
    if [ -n "$STALE" ]; then
        echo
        echo "--- retention: keeping the $KEEP newest cards, removing $(echo "$STALE" | wc -l) ---"
        echo "$STALE" | while IFS= read -r old; do
            [ -n "$old" ] || continue
            # The describe is the field between the version and the arch:
            # rivet-<version>-<describe>[-debug]-aarch64-rpi4-card.img
            #
            # The version is stripped as a FIELD rather than matched literally.
            # It used to be spelled `0.0.1` here, which worked only while the
            # number never moved; ADR-027 §10 makes every image build increment
            # it, so the images this loop removes now carry OLDER versions than
            # the one that just built. A literal would have matched nothing,
            # stripped no prefix, and left every out/debug/ tree on disk — the
            # silent half of the failure, since the images would still go.
            #
            # The trailing `-debug` goes too: $UNSTRIPPED is out/debug/$DESCRIBE
            # with no suffix, so a debug build's symbols were never matched
            # either. That one has been true since the retention was written.
            base=$(basename "$old" -aarch64-rpi4-card.img)
            case "$base" in
                rivet-*-*) desc=${base#rivet-}; desc=${desc#*-}; desc=${desc%-debug} ;;
                *)         desc= ;;
            esac
            printf '    %s (%s)\n' "$(basename "$old")" "$(du -h "$old" | cut -f1)"
            rm -f "$old"
            if [ -n "$desc" ] && [ -d "$REPO/out/debug/$desc" ]; then
                printf '    out/debug/%s (%s)\n' "$desc" "$(du -sh "$REPO/out/debug/$desc" | cut -f1)"
                rm -rf "${REPO:?}/out/debug/$desc"
            fi
        done
        echo
        echo "    Set RIVET_KEEP to change this, or RIVET_KEEP=0 to keep every build."
        echo "    An update payload built from a removed card cannot be rebuilt:"
        echo "    tools/update/build.sh reads the image, not the commit."
    fi
fi
