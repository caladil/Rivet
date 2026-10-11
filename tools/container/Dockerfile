# The card build, in a box — so it runs the same on macOS, Windows and Linux.
#
# `tools/card/build.sh` works, and it needs `sgdisk`, `mkfs.vfat`, `mcopy`,
# `mkfs.erofs`, `mkfs.ext4`, `tar`, `find` and `touch` — `cmd/rivet-image`'s
# `REQUIRED_TOOLS`, checked before a byte is written. On this project's Windows
# workstation those live **only** in a hand-built WSL distro, which is a
# dependency nobody can reproduce from the repository and which `AGENTS.md` §10
# has had to document three separate hazards around (CRLF, `git describe`, path
# rewriting). This image is that distro, written down.
#
# ## It needs no privilege, and that is the point
#
# `cmd/rivet-image/src/disk.rs`'s header: *"No `losetup`, no `mount`, no
# `sudo`."* `mkfs.vfat --offset` and `mkfs.ext4 -E offset=` write a filesystem
# into the middle of a plain file, `mcopy` populates a FAT through mtools'
# `image@@offset` syntax, and a `/system` tree claims root ownership by being
# handed to `mke2fs -d` as a tar built `--owner=0 --group=0`. That crate has a
# test asserting the strings `losetup`, `mount`, `sudo`, `kpartx` and
# `partprobe` appear nowhere in it.
#
# So this container runs with **no `--privileged`, no `--cap-add`, no
# `/dev/loop` and no `--device`**, and it runs as an ordinary uid. That is the
# difference between a build that works everywhere and one that works with
# caveats, and it is a property to preserve rather than a coincidence. If a
# future change to this file reaches for a loop device, the wrong turn was
# several steps earlier.
#
# ## What is NOT in here, deliberately
#
# **Buildroot.** `tools/buildroot/build.sh` produces the kernel, the cross
# toolchain and the C mechanisms of `ADR-009` §6, takes hours, and is a separate
# question. A prebuilt output tree is **mounted in** at `/br` and
# `RIVET_BR_OUTPUT` points at it. The cross `strip` and the cross `gcc` this
# build needs (`ADR-005` §7.5 and `ADR-009` §3.7.4) come from there, so this
# image carries no cross toolchain of its own and does not have to be rebuilt
# when the Buildroot pin moves.
#
# ### Baking that subset in instead — measured, in case somebody tries
#
# It is a fair question, because a self-contained image would need no
# host-specific mount at all. The numbers, taken from the 2026.02.3 tree on
# 2026-08-29:
#
# | What `tools/card/build.sh` reads | Size |
# | :-- | --: |
# | `target/usr/bin/git`, `target/usr/libexec/git-core/git-http-backend` | 6.7 MB |
# | the cross toolchain reached through `host/bin` | ~460 MB |
#
# **`host/bin/aarch64-linux-gcc` and `aarch64-linux-strip` are symlinks**, not
# binaries: this configuration sets `BR2_TOOLCHAIN_EXTERNAL_BOOTLIN`, so the
# real `cc1`, `as`, `ld` and sysroot live in `host/opt/ext-toolchain`, which is
# **387 MB** on its own. `host/bin` being 4.5 MB says nothing about the cost.
#
# The three target binaries are cheap and are genuinely this project's build
# products. The toolchain is not, and it does not have to be baked from anyone's
# private tree: it is **Bootlin `aarch64--musl--stable-2025.08-1`**, a published
# release from <https://toolchains.bootlin.com/>. An image that fetched that
# tarball by URL and checked it against a recorded sha256 would be
# self-contained AND have better provenance than a copy of one developer's
# `host/` directory, which is the "build that lives in one person's scrollback"
# `ADR-026` §6 argues against, merely relocated into a container layer.
#
# That is the shape a future self-contained image should take. It is not done
# here because it has not been proven to produce the same bytes as the tree the
# cards on the board were built with, and an unverified change to the compiler
# is the last place to guess.
#
# **`git describe`.** `ADR-027` §10 makes the version and the commit a 1:1
# mapping and `AGENTS.md` §10.5 says only Windows `git` can read this working
# copy's dirtiness correctly. A `git describe` run *inside* here would be the
# WSL answer wearing a new hat, and would stamp `-dirty` on every image for
# ever. `RIVET_DESCRIBE` arrives as an environment variable, from the host,
# where `tools/card/bump.sh` mints it.
#
# ## Why the versions are pinned twice
#
# `disk.rs` measures byte-for-byte reproducibility against **specific** tool
# versions — gptfdisk 1.0.10, dosfstools 4.2, mtools 4.0.49, erofs-utils 1.9.3,
# e2fsprogs 1.47.4 — and a container that floated its base would quietly stop
# producing the bytes that table describes. `ADR-026` §6 is the general
# argument: a build nobody else can reproduce is a build nobody can review.
#
# So the base is pinned **by digest**, and the packages come from the Arch Linux
# Archive at a fixed date rather than from today's mirrors. Pinning only the
# base would not be enough: `pacman -Sy` against a live mirror resolves to
# whatever shipped this morning, so the image would drift while its `FROM` line
# stood still.
#
# ## Rust
#
# `rustup`, pinned to an exact release, with `aarch64-unknown-linux-musl`.
# Rivet's own binaries need no cross *linker* — `.cargo/config.toml` sets
# `rust-lld`, which ships with the toolchain (`ADR-026` §3.1) — but they do need
# a **host** C toolchain, because build scripts are host programs and `ring`'s
# compiles C. `gcc` is here for that; the cross compiler is Buildroot's.
#
# ## What this does NOT make identical to a build on the host
#
# Say it here rather than let somebody discover it: `rustc` embeds the path of
# every source file it compiles into panic locations, and those paths include
# the workspace root and `CARGO_HOME`. Both are **fixed constants** in here
# (`/rivet` and `/cargo`), which is what makes one container build match another
# container build on a different operating system. It is also what makes a
# container build differ from a build run directly on a host, whose paths are
# that host's. Neither is more correct; the container's is the one that is the
# same everywhere.

# archlinux:base-20260823.0.578598, by digest. The tag is carried alongside so
# a reader can see what it was; the digest is what Docker actually resolves.
FROM archlinux:base-20260823.0.578598@sha256:b860afd5823683f7ea389ba5f00d812f4fe55f6f286dea329d2abeefa535e309

# The Arch Linux Archive snapshot the packages come from. Keep it at or before
# the base image's date — the base carries a keyring from that day, and a
# snapshot newer than the keyring can fail signature checks on packages signed
# by a key the image has never seen.
ARG ARCH_SNAPSHOT=2026/08/23

# The exact rustc this workspace builds with, the same one `rust-toolchain.toml`
# pins for the host. A different one compiles different bytes, so this is part
# of the pin and not a convenience.
ARG RUST_VERSION=1.99.0

# The dependency gate, pinned the same way — see "cargo-deny" below. Matches
# the version already installed and run against this workspace on the host.
ARG CARGO_DENY_VERSION=0.20.2

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# --- the tools disk.rs names, at the versions it measured ---------------------
#
# `sgdisk` gptfdisk, `mkfs.vfat` dosfstools, `mcopy` mtools, `mkfs.erofs`
# erofs-utils, `mkfs.ext4` **and `debugfs`** e2fsprogs. `debugfs` is not in
# REQUIRED_TOOLS and is needed anyway: `tools/card/build.sh` reads the finished
# image back with it and refuses a card missing a binary, which is the check
# that stands between a green build and a board discovered broken later.
#
# `qemu-system-aarch64` runs `tools/qemu/` — the inner loop. It is here since
# 2026-08-30, when the maintainer ruled that this project is Docker and
# platform-independent rather than WSL and platform-dependent (`ADR-026` §7).
# The emulator was the second of two things that still needed a hand-built WSL
# distro; the first was the signing seed, and neither is a reason to keep a
# dependency nobody else can reproduce.
#
# It is NOT small — QEMU pulls a large dependency closure — and that is the
# price of the inner loop running the same way on every machine. Weigh it
# against what it replaces: four card builds and four trial boots were spent in
# one day on questions a smoke test answers in seconds, because the harness
# could not reach a guest at all (`PLAN.md` §102.5).
#
# `tar`, `findutils`, `coreutils`, `sed`, `gawk` and `grep` are in `base`.
# `gcc` links host build scripts. `curl` fetches rustup. `git` is here because
# `cmd/rivet-fleet` names a git dependency and Cargo resolves the whole
# workspace even when `--bin` selects part of it.
#
# **`cpio` goes with it**, and it is the half easy to leave out: `qemu-system-
# aarch64` boots an image and `tools/qemu/run.sh` is what MAKES one, staging a
# tree and packing it with `cpio -o -H newc | gzip`. An image with the emulator
# and not the archiver fails at the first run with `cpio: not found`, which
# reads as a broken harness rather than a missing package.
#
# **No `fakeroot`.** `run.sh` needs root *or* fakeroot to `mknod /dev/console`,
# without which the kernel gives init no stdio and a working boot is
# indistinguishable from a hang (`ADR-026` §3). This container runs as root on
# the platform it is run on here — the `--user` in
# `tools/container/build-card.sh` is Linux-only — so a Linux host running as its
# own uid is the case that would want fakeroot. It is not installed because it
# has not been needed, and `run.sh` refuses by name when neither is available:
# a named refusal rather than a silent difference between hosts.
#
# **`libdrm` is the HOST one, and it is here so the SDK's screen code can be
# compiled and unit-tested at all.** `rivet-sdk`'s `screen` feature links
# `-ldrm`, so every consumer of it — `aura-probe`, `mesa-demo`,
# `template-kiosk` — failed to link for x86_64 with `unable to find library
# -ldrm`, which `tools/test-all.sh` reported as a test failure on 2026-09-03.
# The aarch64 `libdrm.a` a card links comes from Buildroot at `/br` and is
# untouched by this; this copy exists only so `cargo test` can build a host
# binary. A crate that needs a real DRM device still cannot RUN here, and says
# so itself.
#
# **`mesa` is the same argument one feature later, added 2026-09-21.** The
# SDK's `composited-gpu` feature links `-lEGL`, `-lGLESv2` and `-lgbm`, and
# Arch ships all three in `mesa`. Without it `cargo test --workspace
# --all-features` could not LINK — `unable to find library -lgbm` — and when
# `aura-renderer` took that feature to draw GPU backgrounds (`.rulings/`
# R-206), its own 81 tests stopped building too. A feature whose consumers
# cannot be unit-tested in the only Linux this project has is a feature nobody
# is checking.
#
# Same terms as `libdrm` above: the aarch64 libraries a card links come from
# Buildroot at `/br` and are untouched by this. **Nothing here can RUN a GPU
# test** — there is no render node in this container and `gpu::take()` says so
# by name. This makes the code compile and its unit tests link, which is what
# was lost.
#
# **`dbus` is here for the same reason and for an SDK ADOPTER rather than for
# the SDK.** `libdbus-sys` runs a build script that looks for the host D-Bus
# development files, and `btleplug` — the Bluetooth stack TempMon uses — depends
# on it on Linux. Without this package `cargo test --workspace` inside TempMon
# fails at `libdbus-sys` before compiling a line of anything, which is not a
# defect in TempMon and reads exactly like one.
#
# It matters because `exec.sh`'s `RIVET_APP_SRC` makes this container the
# Linux environment an adopter is verified in, and `PLAN.md` §146.4 makes a
# Linux build a precondition of publishing anything. A precondition that cannot
# be evaluated in the only Linux this project has is not a precondition.
#
# On Arch the library and its headers are one package; there is no `-dev` split.
# Nothing on a Rivet card links D-Bus — Rivet has no message bus (`ADR-017`) —
# so this is a host build dependency and reaches no image.
#
# **`pkgconf` is the other half and installing only `dbus` proved it.** Arch
# ships `dbus-1.pc` in that package and ships no `pkg-config` binary anywhere in
# this image, so the first build after adding `dbus` failed with exactly the
# error it had before — `libdbus-sys` looks the library up through `pkg-config`
# and cannot see a `.pc` file without it. `pkgconf` is Arch's provider of that
# command. Nothing else in this image had ever needed one, which is why its
# absence went unnoticed until a crate outside this repository was built here.
#
# **`libxkbcommon` is `mesa`'s argument again, for the compositor.**
# `rivet-displayd` is built on Smithay, which links `-lxkbcommon` with no
# feature to turn it off, so the compositor's host test binary could not LINK
# here (`unable to find library -lxkbcommon`) and its tests stopped running.
# The aarch64 library a card links comes from Buildroot (`BR2_PACKAGE_LIBXKBCOMMON`)
# and is untouched by this; this copy exists only so `cargo test` can build a
# host binary.
#
# **`DisableSandboxSyscalls` is pacman's own switch, shipped commented out.**
# Its downloader sandbox needs seccomp, which Rosetta does not provide, so on an
# arm64 host every package step refuses without it — `ADR-062` §6. Packages are
# still verified against the archive's signatures, and the finished image is
# the same; the `grep` fails the build if a pacman.conf ever stops carrying it.
RUN printf 'Server=https://archive.archlinux.org/repos/%s/$repo/os/$arch\n' "${ARCH_SNAPSHOT}" \
        > /etc/pacman.d/mirrorlist \
    && sed -i 's/^#DisableSandboxSyscalls$/DisableSandboxSyscalls/' /etc/pacman.conf \
    && grep -qx DisableSandboxSyscalls /etc/pacman.conf \
    && pacman -Syu --noconfirm --needed \
        gptfdisk \
        dosfstools \
        mtools \
        erofs-utils \
        e2fsprogs \
        tar \
        findutils \
        gcc \
        curl \
        git \
        cpio \
        qemu-system-aarch64 \
        libdrm \
        mesa \
        dbus \
        pkgconf \
        libxkbcommon \
    && pacman -Scc --noconfirm \
    && rm -rf /var/cache/pacman/pkg/* /var/lib/pacman/sync/*

# The versions, recorded in the build log rather than assumed. `disk.rs`'s
# reproducibility table names five of these; if the snapshot above ever moves,
# this is where the disagreement becomes visible.
RUN set -eu; \
    echo "--- tool versions, for the record ---"; \
    pacman -Q gptfdisk dosfstools mtools erofs-utils e2fsprogs tar cpio qemu-system-aarch64; \
    for t in sgdisk mkfs.vfat mcopy mkfs.erofs mkfs.ext4 tar find touch debugfs; do \
        command -v "$t" >/dev/null || { echo "MISSING: $t" >&2; exit 1; }; \
    done; \
    echo "--- every tool cmd/rivet-image requires is present ---"; \
    for t in qemu-system-aarch64 cpio gzip timeout strings file od; do \
        command -v "$t" >/dev/null || { echo "MISSING: $t" >&2; exit 1; }; \
    done; \
    qemu-system-aarch64 --version | head -1; \
    echo "--- every tool tools/qemu/run.sh requires is present ---"

# --- Rust ---------------------------------------------------------------------
#
# `CARGO_HOME` and `RUSTUP_HOME` are at the filesystem root on purpose: they are
# mount points for a cache volume, and a path under a home directory would be a
# path that changes with the uid the container is run as. Nothing here assumes
# a home directory exists, because the runner may pass any `--user`.
ENV RUSTUP_HOME=/rustup \
    CARGO_HOME=/cargo \
    CARGO_TARGET_DIR=/target \
    PATH=/cargo/bin:/usr/local/bin:/usr/bin:/bin

# **`clippy` is a COMPONENT on top of `minimal`, and its absence was a hole in
# the loop rather than a missing convenience.**
#
# `--profile minimal` is right — this image does not want `rust-docs` or
# `rust-analyzer` — but it also leaves out clippy, and the consequence was worse
# than not having it. `cargo clippy` inside this container answered
# *"'cargo-clippy' is not installed for the toolchain"* on stderr and produced
# **no warnings on stdout**, so a run here read as a clean one. `SPEC-04` §12.2
# recorded that a warning count from this container is zero whether or not there
# are warnings.
#
# There were 287, and three of them were ERRORS that made
# `cargo clippy --workspace --all-targets` exit 101 — invisible to the only loop
# that runs everything.
#
# **`rustfmt` joined it on 2026-09-03, for the same reason and after the same
# discovery.** `tools/test-all.sh` was written to run formatting, lints and
# tests across the root workspace and every nested one, and `cargo fmt` in this
# image answered *"'cargo-fmt' is not installed for the toolchain"*. A runner
# that reports "formatting: not checked" is honest; one that reports nothing is
# the 287 warnings again. The component costs a few megabytes and removes a
# whole class of skip.
RUN set -eu; \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal \
            --default-toolchain "${RUST_VERSION}" \
            --component clippy \
            --component rustfmt \
            --target aarch64-unknown-linux-musl; \
    rustc --version; \
    cargo --version; \
    # Checked here rather than discovered by a lint run that says nothing. A
    # component that failed to install is a silent gate, which is the state
    # this line exists to make impossible.
    cargo clippy --version; \
    cargo fmt --version; \
    # `rust-lld` is what .cargo/config.toml names as the target linker. It ships
    # inside the toolchain rather than on PATH, so its absence would surface as
    # a link failure deep in a dependency; found here instead.
    find /rustup/toolchains -name 'rust-lld' -type f -print -quit | grep -q . \
        || { echo "no rust-lld in the toolchain — .cargo/config.toml names it" >&2; exit 1; }

# **`cargo-deny` is the dependency gate `deny.toml` configures (`ADR-010`
# §4.1), and it is not a rustup component — it is an ordinary crates.io crate,
# installed with `cargo install` once the toolchain above exists.** Pinned to
# an exact release with `--locked` so its own dependency tree cannot drift
# independently of everything else pinned in this file, matching the version
# already run against this workspace on the host.
#
# Before this, `tools/test-all.sh` reported "NOT CHECKED HERE: dependencies —
# no cargo-deny in this toolchain" on every run made inside this container,
# and the gate `licenses`, `bans` and `sources` depend on ran only on whichever
# workstation happened to have the binary installed by hand.
RUN set -eu; \
    cargo install cargo-deny --version "${CARGO_DENY_VERSION}" --locked; \
    cargo-deny --version

# The rustup shims live in $CARGO_HOME/bin, and $CARGO_HOME is about to have a
# volume mounted over it. A named volume is seeded from the image, so they would
# normally survive — but a volume created against an *older* image is not
# reseeded, and the failure mode is `cargo: not found` with no clue why. Copies
# outside the mount point make that impossible.
#
# **The component shims are in this list too, added 2026-09-03 with `rustfmt`.**
# `cargo fmt` and `cargo clippy` are `cargo` looking for `cargo-fmt` and
# `cargo-clippy` **on PATH**, and both live only in `/cargo/bin`. An existing
# `rivet-card-cargo` volume was created against an older image and is therefore
# NOT reseeded when this one is built — so a newly added component would be
# absent from `/cargo/bin` on exactly the machines that already have the
# project set up. `PATH` puts `/cargo/bin` first and `/usr/local/bin` second, so
# a copy here is what makes the volume's age stop mattering.
#
# **`cargo-deny` joined the list on 2026-09-25, for the identical reason.**
# `cargo install` above put it in `/cargo/bin` same as any other cargo
# subcommand, which is exactly the path an old volume shadows.
RUN set -eu; \
    for b in cargo rustc rustup rustdoc cargo-fmt rustfmt cargo-clippy clippy-driver cargo-deny; do \
        cp -a "/cargo/bin/$b" "/usr/local/bin/$b"; \
    done; \
    /usr/local/bin/cargo --version >/dev/null; \
    PATH=/usr/local/bin:/usr/bin:/bin /usr/local/bin/cargo fmt --version >/dev/null; \
    PATH=/usr/local/bin:/usr/bin:/bin /usr/local/bin/cargo clippy --version >/dev/null; \
    /usr/local/bin/cargo-deny --version >/dev/null

# Writable by whatever uid the runner passes. The container writes nothing that
# needs to be owned by anyone in particular: `--all-root` and a `--owner=0` tar
# are how the image gets root-owned files (disk.rs's header), not the builder's
# identity.
RUN mkdir -p /cargo /rustup /target /rivet /br /out \
    && chmod 0777 /cargo /target /out /tmp

WORKDIR /rivet

# No ENTRYPOINT. The runner names `tools/card/build.sh` explicitly, so what runs
# is legible in the `docker run` line and an operator can substitute a shell to
# look around without fighting an entrypoint.
CMD ["tools/card/build.sh"]
