# Required by Buildroot's br2-external mechanism. ~~Deliberately empty, for the
# reason given in Config.in — Rivet contributes configuration to Buildroot,
# never build logic (`ADR-009` §6).~~
#
# **One line since 2026-08-30, and it is an ORDERING fact rather than build
# logic**, which is the distinction §6 draws. `linux/linux.mk` says it itself:
#
#   > The kernel CONFIG_EXTRA_FIRMWARE feature requires firmware files at build
#   > time. Make sure they are available before the kernel builds.
#
# and then lists four firmware packages the kernel waits for. The Raspberry Pi's
# is not among them, so with `CONFIG_EXTRA_FIRMWARE` naming files that
# `brcmfmac_sdio-firmware-rpi` installs, the kernel can be built before they
# exist — and a kernel built without them fails silently, carrying no firmware
# and saying nothing.
#
# **Why the firmware has to be IN the kernel at all** (`PLAN.md` §102.25.5):
# it is requested 119 ms before the root filesystem is mounted. Measured on the
# board — `Direct firmware load … failed with error -2` at 2.7515 s, `VFS:
# Mounted root` at 2.8703 s. The file was on the card, at the right path, and
# did not exist yet. Raspberry Pi OS does not meet this because `brcmfmac` is a
# module udev loads long after root is up; Rivet has no initramfs, no udev and
# no modules, so the only place firmware can be is the kernel image.
LINUX_DEPENDENCIES += \
	$(if $(BR2_PACKAGE_BRCMFMAC_SDIO_FIRMWARE_RPI),brcmfmac_sdio-firmware-rpi)
