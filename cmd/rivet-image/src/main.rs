//! Assembles a bootable Rivet card.
//!
//! `ADR-009` §6 drew the line this crate sits on: **Buildroot builds the C
//! mechanisms and the toolchain; Rivet's own image assembly — `ADR-008`'s
//! layout, the generated `/etc`, the A/B structure — stays in Rivet's own
//! tooling.** This is that tooling.
//!
//! **Every binary this image installs is one this workspace compiles**, and
//! that is why the card build needs no Buildroot output of its own. The
//! boundary `ADR-009` §6 draws is still visible in this file's shape, though:
//! a binary arrives as **a path to a finished file**, exactly as the firmware
//! and the kernel do, and nothing about how it was built is this crate's
//! business. `wpa_supplicant` is Stage 2 and is absent.
//!
//! `rivet-timed` (`ADR-021` §7) is the input whose consumer was already on the
//! card before the input was: `cmd/rivet-sysd` had defined the service,
//! disabled, with "the image does not install this binary" as the entire stated
//! reason. See [`system::TIMED_PATH`].
//!
//! ## This is a host tool
//!
//! It runs on the build machine, not on the Pi, and it drives `sgdisk`,
//! `mkfs.vfat`, `mkfs.ext4` and `mcopy` rather than reimplementing them. That
//! is `ADR-009` §3.4's left column — self-contained programs driven over a
//! pipe, whose types never reach Rivet's own — and it is the same judgement
//! that made `wpa_supplicant` a mechanism rather than a rewrite.
//!
//! **It needs no privilege.** `mkfs` writes at an offset into the image file
//! and `mke2fs -d` takes a tarball that carries root ownership, so there is no
//! `losetup`, no `mount` and no `sudo` anywhere in the path.
//!
//! ## What it produces
//!
//! A disk **image file**, not a written card. Writing is `dd` or Raspberry Pi
//! Imager, and keeping those separate means the risky step is one the operator
//! takes deliberately, with a device path they typed themselves.
//!
//! ## The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`layout`] | `ADR-018` §4's partition table as data. The shared contract; no I/O |
//! | [`boot`] | what goes on a FAT boot partition — `ADR-002` §4 |
//! | [`system`] | the `/system` tree — `ADR-008` §3 |
//! | [`disk`] | partitioning, `mkfs`, and assembling the image |
//! | [`fleet_payloads`] | resolving a role's required payloads by digest, from `--fleet-repository` |
//! | `main` | the composition root: read inputs, hand them over, report. No logic |

mod application;
mod boot;
mod disk;
mod fleet_payloads;
mod layout;
mod system;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use layout::Slot;

/// Where the inputs live, relative to the repository root.
///
/// `tools/fetch-firmware.sh` puts the firmware in `vendor/boot`, and
/// `tools/qemu/fetch-kernel.sh` puts the kernel and device tree in
/// `tools/qemu`. Both are gitignored: `ADR-010` §6 says the Raspberry Pi
/// firmware is redistributable but is not this project's to carry.
const FIRMWARE_DIR: &str = "vendor/boot";

/// Where a kernel is read from when `--kernel` was not given.
///
/// # This is a FALLBACK and it was the default for far too long
///
/// `tools/qemu/fetch-kernel.sh` copies a kernel **off a running Raspberry Pi
/// OS board**, and until 2026-08-30 that was the only kernel `cmd/rivet-image`
/// could use — so every Rivet card ever built booted Raspberry Pi's kernel
/// rather than the one `ADR-024` §5 rules Buildroot builds.
///
/// **What that cost was measured**: in Raspberry Pi OS's kernel the wireless
/// stack is modular, a Rivet image has no `/lib/modules`, and the board
/// answered `/sys/bus/sdio/drivers` with `entry_count: 0` — no driver on the
/// bus at all, which is why `wlan0` never existed and why nothing ever printed
/// a reason. `PLAN.md` §102.25.
///
/// It is kept because it still boots and is a way back if Buildroot's kernel
/// does not, but a build that uses it is using somebody else's kernel.
const KERNEL_DIR: &str = "tools/qemu";

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("rivet-image: {message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    format!(
        "usage: rivet-image OUT.img INIT SUPERVISOR CLI NETD TIMED MDNSD\n\
         \x20                    GIT HTTP-BACKEND DEPLOY MCPD\n\
         \x20                    [--nushell PATH] [--femto PATH] [--splash PATH]\n\
         \x20                    [--bluetoothd PATH]\n\
         \x20                    [--ca-bundle PATH] [--zoneinfo DIR]\n\
         \x20                    [--gdbserver PATH] [--strace PATH] [--device-mib N] [--default-slot a|b]\n\
         \x20                    [--document PATH] [--fleet-repository DIR] [--identity DIR]\n\
         \x20                    [--staging PATH] [--launcher PATH] [--displayd PATH]\n\
         \x20                    [--proxyd PATH] [--dnsd PATH] [--application PATH]...\n\
         \n\
         \x20 OUT.img      the image file to write\n\
         \x20 INIT         the rivet-initd binary, static aarch64\n\
         \x20 SUPERVISOR   the rivet-sysd binary, static aarch64\n\
         \x20 CLI          the rivet binary, static aarch64 — also a login shell\n\
         \x20 NETD         the rivet-netd binary, static aarch64\n\
         \x20 TIMED        the rivet-timed binary, static aarch64 — ADR-021 §7's\n\
         \x20              SNTP client. The supervisor's service definition has\n\
         \x20              named this path since before a card carried it\n\
         \x20 MDNSD        the rivet-mdnsd binary, static aarch64 — PLAN.md §23s\n\
         \x20              responder. The supervisor names this path and ships the\n\
         \x20              service DISABLED, so the card carries it and does not\n\
         \x20              run it until the document says so\n\
         \x20 GIT          the git binary, static aarch64 — NOT built by this\n\
         \x20              workspace. tools/buildroot/build.sh leaves it at\n\
         \x20              <output>/target/usr/bin/git (ADR-009 §6). ONE file:\n\
         \x20              the git-core directory beside it is 147 hardlinks to\n\
         \x20              this same inode and every verb Rivet needs is a\n\
         \x20              builtin reached as `git <verb>` (PLAN.md §43.6)\n\
         \x20 HTTP-BACKEND git-http-backend, static aarch64 - ADR-035 4s smart HTTP.\n\
         \x20              NOT built by this workspace and NOT the same file as GIT:\n\
         \x20              buildroot leaves it at target/usr/libexec/git-core/, and\n\
         \x20              PLAN.md 84.1 measured the inodes to prove it is its own\n\
         \x20              binary rather than a builtin. It needs /system/bin on PATH\n\
         \x20              at RUNTIME or it serves empty repositories (PLAN.md 85.2).\n\
         \x20 DEPLOY       the rivet-deploy binary, static aarch64 — ADR-029 §14's\n\
         \x20              installer. NOTHING ON THE CARD STARTS IT: how a payload\n\
         \x20              reaches it is not ruled (the maintainer's 2026-08-24 install-request ruling). Required anyway,\n\
         \x20              because a card that cannot install an update is not a\n\
         \x20              variant, it is a card that can never be updated\n\
         \x20 MCPD         the rivet-mcpd binary, static aarch64 — ADR-035s control\n\
         \x20              plane, and since phase 3 the WHOLE administration\n\
         \x20              surface. The supervisor names this path and ships the\n\
         \x20              service ENABLED (ADR-023 4.1), so a card built without\n\
         \x20              it boots to a board nothing can reach\n\
         \n\
         \x20 --identity DIR the device's TLS identity, as `cargo rivet identity new`\n\
         \x20                writes it: device.seed, device.crt and a peers file\n\
         \x20                beside them. ALL THREE OR NONE. They go on the BOOT\n\
         \x20                partition; PID 1 copies them to /state/secrets on first\n\
         \x20                boot and DELETES the seed from the card (ADR-019 3.1).\n\
         \x20                A card without one boots, and its control plane refuses\n\
         \x20                to start -- visible in `rivet get status`, costing no\n\
         \x20                other service.\n\
         \x20 --staging PATH the staging bundle rivet-provision writes: the\n\
         \x20                certificates, keys and fleet register an APPLICATION\n\
         \x20                on this machine is handed before it first starts.\n\
         \x20                Same journey as --identity -- it goes on the BOOT\n\
         \x20                partition, PID 1 copies it to /state/staging and\n\
         \x20                DELETES it from the card, and the SUPERVISOR then\n\
         \x20                delivers each application's share into the data\n\
         \x20                directory it owns, with that application's uid. The\n\
         \x20                extra hop is the point: the uid is derived at\n\
         \x20                discovery and is not knowable here (ADR-049).\n\
         \x20                The file is PARSED before the card is written, so a\n\
         \x20                bundle that will not read is refused on your\n\
         \x20                workstation rather than on a headless board.\n\
         \x20 --application PATH\n\
         \x20                a signed .rvt this card CARRIES -- ADR-050 4.\n\
         \x20                REPEATABLE; each names one payload. It rides on the\n\
         \x20                BOOT partition beside the document, the identity and\n\
         \x20                the staging bundle; PID 1 lands it in /state/carried\n\
         \x20                on the first boot and the supervisor asks\n\
         \x20                rivet-deploy to install it, which is the same route\n\
         \x20                `cargo rivet deploy` takes afterwards.\n\
         \x20                VERIFIED HERE, against the trust store THIS image is\n\
         \x20                about to carry -- so a payload signed by a key this\n\
         \x20                card would refuse is refused on your workstation\n\
         \x20                rather than on a headless board. The name on the card\n\
         \x20                comes from the payload's SIGNED header, not from the\n\
         \x20                file you named.\n\
         \x20                NOT SECRET and NOT deleted from the card: a .rvt is\n\
         \x20                protected by its signature, so the card keeps saying\n\
         \x20                what it was built to carry.\n\
         \x20 --launcher PATH\n\
         \x20                the rivet-launcher binary, static aarch64 -- ADR-050\n\
         \x20                2.1's tier 2. OPTIONAL, and the service ships\n\
         \x20                DISABLED like rivet-bluetoothd's, so a card carrying\n\
         \x20                it shows no menu until the document says so. A card\n\
         \x20                without it is a supported headless machine.\n\
         \x20 --displayd PATH\n\
         \x20                the rivet-displayd binary -- ADR-058 4.1's\n\
         \x20                compositor, on the launcher's tier and its terms.\n\
         \x20                OPTIONAL. It is DYNAMIC, and the only binary of\n\
         \x20                Rivet's own that is, so a card carrying it must\n\
         \x20                also carry libgbm, libxkbcommon and libgcc_s.\n\
         \x20 --proxyd PATH\n\
         \x20                the rivet-proxyd binary, static aarch64 -- ADR-060's\n\
         \x20                front door, part of the operating system and run under\n\
         \x20                its own uid and Landlock ruleset. OPTIONAL here, and\n\
         \x20                the service ships DISABLED: the document's role turns\n\
         \x20                it on. tools/card/build.sh always passes it.\n\
         \x20 --dnsd PATH\n\
         \x20                the rivet-dnsd binary, static aarch64 -- ADR-047's name\n\
         \x20                service. OPTIONAL, and the service ships DISABLED: a\n\
         \x20                document with `[services.rivet-dnsd] enabled = true`\n\
         \x20                and a `[dns]` table turns it on.\n\
         \x20 --kernel PATH       the kernel for the boot partition, as an\n\
         \x20                       uncompressed arm64 Image. Buildroot leaves it\n\
         \x20                       at <output>/images/Image. WITHOUT THIS the build\n\
         \x20                       falls back to tools/qemu/kernel8.img, which\n\
         \x20                       fetch-kernel.sh copied off a Raspberry Pi OS\n\
         \x20                       board - somebody else's kernel, whose modular\n\
         \x20                       wireless stack cannot load on a Rivet image\n\
         \x20                       (PLAN.md §102.25). Goes with --device-tree.\n\
         \x20 --device-tree PATH    the device tree for that kernel, Buildroot's\n\
         \x20                       <output>/images/bcm2711-rpi-4-b.dtb. Required\n\
         \x20                       with --kernel and refused without it.\n\
         \x20 --supplicant PATH     wpa_supplicant, static aarch64 - ADR-004 §3's\n\
         \x20                       PERMANENT Tier C dependency. Buildroot leaves it\n\
         \x20                       at <output>/target/usr/sbin/wpa_supplicant.\n\
         \x20                       Omit it for a card with no wireless, which is\n\
         \x20                       a supported machine.\n\
         \x20 --wpa-cli PATH        wpa_cli. A BRING-UP tool: it proves the C side\n\
         \x20                       associates independently of rivet-netd, so a\n\
         \x20                       failure has one candidate cause not two.\n\
         \x20 --udhcpc PATH         the DHCP client. Buildroot leaves it at\n\
         \x20                       <output>/target/bin/busybox, built with exactly\n\
         \x20                       ONE applet; it is installed as `udhcpc` because\n\
         \x20                       that is what it is.\n\
         \x20 --wifi-firmware DIR   the CYW43455 firmware directory, Buildroot's\n\
         \x20                       <output>/target/lib/firmware/brcm. FOUR files\n\
         \x20                       are taken: three for the WIRELESS half by the\n\
         \x20                       names ADR-004 §8.1 measured, and one for the\n\
         \x20                       BLUETOOTH half added by R-144 — the flag is\n\
         \x20                       still called --wifi-firmware because it names\n\
         \x20                       one directory, and both halves live in it.\n\
         \x20                       A missing clm_blob does not fail, it makes\n\
         \x20                       association MISBEHAVE; a missing .hcd means no\n\
         \x20                       Bluetooth controller ever appears.\n\
         \x20 --splash PATH  the rivet-splash binary, static aarch64. PID 1 runs it\n\
         \x20                on an ordinary boot and NOT on a trial boot, so a\n\
         \x20                trial shows the console it is being watched for\n\
         \x20 --bluetoothd PATH\n\
         \x20                the rivet-bluetoothd binary, static aarch64 — ADR-041's\n\
         \x20                BLE observer. OPTIONAL, and paired with the KERNEL\n\
         \x20                rather than with a screen: it needs CONFIG_BT, which no\n\
         \x20                kernel built before 2026-09-05 has. The service ships\n\
         \x20                DISABLED, so a card carrying it does not hold the radio\n\
         \x20 --femto PATH   the femto binary, static aarch64 — ADR-005 §5.5.1's\n\
         \x20              editor. A card without one is a supported card.\n\
         \x20 --nushell PATH the nu binary, static aarch64 — ADR-005 §5.1's login\n\
         \x20              shell. OPTIONAL: without it the\n\
         \x20              card's login shell is the CLI, which is every card built\n\
         \x20              before 2026-08-24. tools/nushell/build.sh produces it\n\
         \x20 --gdbserver PATH the gdbserver stub, static aarch64 — ADR-005 §7.\n\
         \x20              OPTIONAL, and whether a card SHOULD carry one is\n\
         \x20              ADR-005 §7.4 and unruled. tools/debug/build.sh\n\
         \x20              produces it; nothing on the card starts it\n\
         \x20 --strace PATH  strace, static aarch64 — ADR-005 §7.2. OPTIONAL, on\n\
         \x20              the same terms\n\
         \x20 --arch TOKEN   REQUIRED. ADR-027 §3's token for the target the\n\
         \x20              binaries were built for: aarch64, armv6 or x86_64.\n\
         \x20              Recorded at /system/arch; the machine refuses every\n\
         \x20              artifact built for anything else\n\
         \x20 --device-mib   size of the card, in MiB (default {})\n\
         \x20 --default-slot which slot a fresh card boots (default a)\n\
         \x20 --document PATH the first-boot provisioning document, written as\n\
         \x20               rivet.toml onto BOTH boot partitions. Without one\n\
         \x20               the card carries a template declaring a schema\n\
         \x20               version and nothing else -- which boots to a machine\n\
         \x20               nobody can reach, because it takes no address.\n\
         \x20               NOT validated here unless --fleet-repository is\n\
         \x20               also given: without it, this crate does not depend\n\
         \x20               on rivet-config's parser, so a bad document is\n\
         \x20               refused by the BOARD on first boot.\n\
         \x20 --fleet-repository DIR\n\
         \x20               resolve --document's enabled application services\n\
         \x20               out of this fleet repository, by digest, and carry\n\
         \x20               them -- PLAN-Feature-Set-2026-09-12 4, ADR-050\n\
         \x20               5.1. Requires --document. Additive with\n\
         \x20               --application, not a replacement for it; either\n\
         \x20               naming a product the other already resolved is the\n\
         \x20               same two-payloads-one-product refusal. Refuses\n\
         \x20               rather than guessing which version to carry: a\n\
         \x20               product with zero or more than one `application`\n\
         \x20               artifact in the manifest is refused by name\n\
         \x20               (AGENTS.md 5).\n\
         \n\
         Produces an image file. It does NOT write a card — do that yourself,\n\
         with a device path you typed, once you have checked it.",
        default_device_mib()
    )
}

/// The smallest card this layout fits on — **the minimum, deliberately.**
///
/// **Changed 2026-08-23, and the reasoning it replaces was sound at the time.**
/// This was `7000`: an 8 GB card less the slack every card of a nominal size
/// lacks. The comment here read *"deliberately not the minimum: a too-small
/// default would silently produce a `/state` of a few megabytes, which boots
/// and then fills."* That hazard is real and has not gone away. What changed is
/// the price of avoiding it that way.
///
/// Sizing the image to the card means **writing the card to deliver the
/// image**, and the two are not the same quantity. A 7000 MiB image of this
/// layout holds about 67 MB of non-zero bytes — 99% of what the operator waits
/// for is zeroes being written over zeroes. Measured 2026-08-23: the whole
/// image compresses to 22.4 MB, and `/state` accounted for 5208 of the 7000
/// MiB while containing nothing at all. Twenty minutes of an operator's card
/// writer, per iteration, to move two binaries totalling about 1 MB.
///
/// So the default is the minimum and `/state` is small **on the card**, which
/// is only correct because `ADR-018` §10 grows it on first boot. Until that
/// exists the old comment's hazard is live, and [`render`] says so out loud on
/// every build rather than leaving it to be discovered when the filesystem
/// fills — `AGENTS.md` §3: refuse or warn, never quietly proceed.
///
/// `ADR-018` §4's ruling is unchanged and is in fact better served: `/state`
/// takes *the remainder of the device*, which a fixed-size image cannot know.
/// Growing on first boot is how a 2 GiB image honours that on a card of any
/// size; sizing the image to one particular card was only ever honouring it for
/// that card.
const fn default_device_mib() -> u64 {
    layout::minimum_device_mib()
}

struct Args {
    image: String,
    init: PathBuf,
    supervisor: PathBuf,
    /// The `rivet` CLI. **Positional and required**, like the other three:
    /// `system::LOGIN_SHELLS` names it, so an image built without it would be
    /// one `system::check_login_shells` refuses — and an optional argument whose
    /// absence is always an error is not an optional argument.
    cli: PathBuf,
    netd: PathBuf,
    /// The `rivet-timed` binary. **Positional and required**, on the same
    /// argument as the five above with one difference worth stating: this is the
    /// only input whose *consumer already exists on the card*.
    /// `cmd/rivet-sysd` has defined the service since 2026-08-23 and
    /// shipped it disabled with "`cmd/rivet-image` does not install this
    /// binary" as the stated and only reason. An optional argument here would
    /// mean a card whose supervisor names a path nothing put there — the
    /// `ENOENT`-on-every-boot failure that definition was written to avoid.
    timed: PathBuf,
    /// The `rivet-mdnsd` binary. **Positional and required**, for exactly
    /// `timed`'s reason: `cmd/rivet-sysd` names this path in a definition it
    /// ships, so an optional argument here would mean a card whose supervisor
    /// names a path nothing put there. It ships DISABLED; that is the
    /// document's decision and is not this argument's.
    mdnsd: PathBuf,
    /// The `git-http-backend` binary. **Positional and required**, on `git`'s
    /// terms: `ADR-035`'s phase 3 moves repositories onto the control plane,
    /// so a card without this is a card whose repositories cannot be reached at
    /// all.
    http_backend: PathBuf,
    /// The `git` binary. **Positional and required**: `cmd/rivet-mcpd`'s smart
    /// HTTP handler execs `git-http-backend`, so an optional argument here
    /// would produce a card that has the account, the keys and the shell, and
    /// fails at the moment of the push.
    ///
    /// **Not built by this workspace**, and for `ADR-009` §6's reason rather
    /// than `nushell`'s — a C mechanism is Buildroot's.
    /// `tools/buildroot/build.sh` leaves it at `target/usr/bin/git`, and the
    /// **one file** is what is wanted; `system::GIT_PATH` has the arithmetic on
    /// why the `git-core` directory beside it is not.
    git: PathBuf,
    /// The `rivet-deploy` binary. **Positional and required**, and the
    /// argument is `ADR-018` §5's rather than any of the six above.
    ///
    /// A card without an installer is not a supported variant: `ADR-018` §5's
    /// two-slot mechanism exists so a machine can be updated, and a card that
    /// cannot verify and write a payload can never take one. That is a reflash
    /// rather than a degraded mode, which is the test every other positional
    /// here is chosen by.
    ///
    /// **Its consumer does not exist yet and that is not a reason to make it
    /// optional** — the opposite of `timed`'s situation, and worth stating
    /// because it looks similar. `cmd/rivet-sysd`'s `definitions::deploy`
    /// is written and nothing calls it, because the maintainer's 2026-08-24
    /// install-request ruling left the channel unruled. A channel cannot be tested against a card that does not carry
    /// the program, so the binary goes on first.
    deploy: PathBuf,
    /// The `nu` binary. **An option, not a positional**, and it is the only
    /// input of the seven that is optional.
    ///
    /// The rule the positionals follow is stated on `cli`: an argument whose
    /// absence is always an error is not an optional argument.
    /// This one inverts cleanly — its absence produces a *correct* card, one
    /// whose login shell is `cmd/rivet`, which is what every card built
    /// before 2026-08-24 was. The ruling that makes `nu` optional requires that card to
    /// keep working, so making this positional would encode the opposite.
    nushell: Option<PathBuf>,
    ca_bundle: Option<PathBuf>,
    system_font: Option<PathBuf>,
    mono_font: Option<PathBuf>,
    icon_font: Option<PathBuf>,
    /// The zone database, already enumerated — see [`zone_database`].
    ///
    /// Empty is a card that can only keep UTC, which is every card built before
    /// 2026-09-02 and is a supported card (`ADR-021` §9).
    zoneinfo: Vec<(String, PathBuf)>,
    /// The `femto` binary. **An option, not a positional**, on `nushell`'s
    /// terms: a card with no editor is a supported card (`system::FEMTO_PATH`).
    femto: Option<PathBuf>,

    /// The `rivet-splash` binary. **An option**, on `femto`'s terms.
    splash: Option<PathBuf>,
    /// The `rivet-bluetoothd` binary. **An option**, and NOT on `splash`'s
    /// terms — `system::BLUETOOTHD_PATH` says why: it is paired with a kernel
    /// rather than with a screen.
    bluetoothd: Option<PathBuf>,
    /// The `wpa_supplicant` binary. **An option**, on `splash`'s terms: a card
    /// with no wireless is a supported machine, because `ADR-004` §2 brings up
    /// wired first and §2.4 keeps the wired interface afterwards.
    supplicant: Option<PathBuf>,
    /// `wpa_cli`. **An option**, and a bring-up tool rather than part of the
    /// machine - `system::WPA_CLI_PATH` says why it is worth carrying anyway.
    wpa_cli: Option<PathBuf>,
    /// The `udhcpc` binary. **An option**: every Rivet board so far has run on
    /// a static address, so a card without a DHCP client is the normal one.
    udhcpc: Option<PathBuf>,
    /// The `rivet-dhcp-hook` binary. **An option on its own and REQUIRED
    /// beside `--udhcpc`**: a client with nothing to exec takes an offer and
    /// then does nothing, which looks exactly like a network that is down.
    dhcp_hook: Option<PathBuf>,
    /// The kernel to put on the boot partition. **An option, and the one that
    /// should always be given** — see `KERNEL_DIR`.
    kernel: Option<PathBuf>,
    /// The device tree to put beside it. Given with `--kernel` or not at all:
    /// a device tree from one kernel and an image from another is the pairing
    /// nobody should be able to make by forgetting a flag.
    device_tree: Option<PathBuf>,
    /// musl's dynamic loader — `system::LOADER_PATH`. **An option**, and a
    /// card without one is a supported card: every Rivet binary is static
    /// (`ADR-003` §5), so this is here for APPLICATIONS. Shared Rivets, 2026-08-31.
    loader: Option<PathBuf>,
    /// Vendor shared libraries for `/lib` — Mesa, 2026-08-31. **Repeatable**,
    /// and each is installed under its own file name, which the caller has
    /// already set to the SONAME.
    libraries: Vec<PathBuf>,
    /// `(destination, source)` for shared objects whose loader has the path
    /// compiled in — Mesa's GBM backend. **Repeatable.**
    libraries_at: Vec<(String, PathBuf)>,
    /// The DIRECTORY holding the CYW43455 firmware. **An option**, and a
    /// directory rather than three paths - `system::FIRMWARE_FILES` knows the
    /// names, which `ADR-004` §8.1 measured off a real board.
    wifi_firmware: Option<PathBuf>,
    /// The directory holding a device's TLS identity, if the operator has one.
    ///
    /// **A directory rather than three paths**, because `cargo rivet identity
    /// new` writes a directory, and the operator's job should be to name it
    /// rather than to re-state its contents -- `ADR-011`: the layout is that
    /// command's.
    identity: Option<PathBuf>,
    /// The staging bundle `cmd/rivet-provision` writes — `ADR-049`.
    ///
    /// **A file rather than a directory**, unlike `identity`, and for the
    /// symmetric reason: an identity's layout belongs to `cargo rivet identity
    /// new` and this bundle's whole content is one document
    /// `rivet_core::staging` defines. Naming a directory here would invite this
    /// crate to invent a second layout for material it only copies.
    staging: Option<PathBuf>,
    /// The signed application payloads this card carries — `ADR-050` §4.
    ///
    /// **Repeatable, and it accumulates**, on `--library`'s terms exactly: it
    /// names a set rather than a file, and a second `--application` that
    /// discarded the first would produce a card missing an application nobody
    /// could see was asked for.
    ///
    /// **A file rather than a directory**, like `staging` and unlike
    /// `identity`: a `.rvt` is one signed artifact and there is no layout for
    /// this crate to invent around it.
    applications: Vec<PathBuf>,
    /// The `rivet-launcher` binary. **An option**, on `bluetoothd`'s terms —
    /// `ADR-050` §2 puts it on tier 2, where the image carries it and the
    /// **document** decides whether it runs.
    ///
    /// A card without one is a supported machine, and it is the ordinary
    /// headless card: `ADR-050` §5's table gives the launcher to the two
    /// kiosk rows and to neither headless one.
    launcher: Option<PathBuf>,
    /// The `rivet-displayd` binary. **An option**, on the launcher's terms
    /// exactly — `ADR-058` §4 puts it on the same tier 2.
    displayd: Option<PathBuf>,
    /// The `rivet-proxyd` binary. **An option**, on the launcher's terms —
    /// `ADR-060` makes it an image service the document's role enables.
    proxyd: Option<PathBuf>,
    /// The `rivet-dnsd` binary. **An option**, on the proxy's terms —
    /// `ADR-047`'s name service, shipped disabled.
    dnsd: Option<PathBuf>,
    /// The `rivet-mcpd` binary. **Positional and required**, on `mdnsd`'s
    /// terms exactly: `cmd/rivet-sysd` names this path in a definition it
    /// ships, so an optional argument here would mean a card whose supervisor
    /// names a path nothing put there. It ships DISABLED, and that is the
    /// roadmap's decision rather than this argument's — `system::MCPD_PATH`
    /// carries why those are different claims.
    mcpd: PathBuf,
    /// `gdbserver`. **An option, and optional for a different reason from
    /// `nushell`'s.**
    ///
    /// `nushell`'s absence is ruled harmless. This
    /// one's absence is the *status quo of an undecided question*: `ADR-005`
    /// §7.4 asks whether a debugger belongs on every card, only on some, or on
    /// none, and that is the maintainer's. Making it required would answer it
    /// by making every existing build command wrong; defaulting it on would
    /// answer it silently, and `ADR-028` is the file that would have wanted a
    /// say. So the flag exists, the default is off, and the ruling stays open.
    gdbserver: Option<PathBuf>,
    /// `strace`, on exactly the terms of [`Args::gdbserver`].
    strace: Option<PathBuf>,
    /// Public keys this image will trust, each with the kinds it may sign.
    ///
    /// `--trusted-key <label>:<kinds>:<path>`, e.g.
    /// `--trusted-key rivet-system:update,extension:/keys/system.pub`.
    /// `ADR-028` §5.5: a key is trusted **for** something, and the label exists
    /// so a refusal can name the key in the terms its holder knows it by.
    trusted_keys: Vec<(String, Vec<String>, PathBuf)>,
    /// Build an image that trusts nobody, on purpose. `ADR-028` §5.5.5.
    development_only: bool,
    /// `--document PATH`: the first-boot provisioning document to write as
    /// `rivet.toml`, instead of `boot::provisioning_toml`'s template.
    ///
    /// See `boot::BootInputs::document` for what a card without one is: a
    /// machine that boots and that nobody can reach.
    document: Option<PathBuf>,
    /// `--fleet-repository DIR`: resolve `--document`'s enabled application
    /// services out of this fleet repository, by digest, and carry them —
    /// `PLAN-Feature-Set-2026-09-12` §4, `ADR-050` §5.1.
    ///
    /// **Additive with `--application`, never a replacement for it.** Both
    /// contribute to the same carried set, and the two-payloads-one-product
    /// refusal below catches either one naming something the other already
    /// did. Requires `--document`: with no document there is no enabled
    /// service to resolve, and refusing that is `AGENTS.md` §3 — refuse
    /// rather than silently carry nothing.
    fleet_repository: Option<PathBuf>,
    /// `--arch`, `ADR-027` §3's token for the target every binary was built
    /// for. **Required** — see `system::Inputs::arch`.
    arch: String,
    device_mib: u64,
    default_slot: Slot,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut device_mib = default_device_mib();
    let mut default_slot = Slot::A;
    let mut nushell: Option<PathBuf> = None;
    let mut ca_bundle: Option<PathBuf> = None;
    let mut zoneinfo: Vec<(String, PathBuf)> = Vec::new();
    let mut system_font: Option<PathBuf> = None;
    let mut mono_font: Option<PathBuf> = None;
    let mut icon_font: Option<PathBuf> = None;
    let mut femto: Option<PathBuf> = None;
    let mut splash: Option<PathBuf> = None;
    let mut bluetoothd: Option<PathBuf> = None;
    let mut supplicant: Option<PathBuf> = None;
    let mut wpa_cli: Option<PathBuf> = None;
    let mut udhcpc: Option<PathBuf> = None;
    let mut dhcp_hook: Option<PathBuf> = None;
    let mut kernel: Option<PathBuf> = None;
    let mut device_tree: Option<PathBuf> = None;
    let mut wifi_firmware: Option<PathBuf> = None;
    let mut loader: Option<PathBuf> = None;
    let mut libraries: Vec<PathBuf> = Vec::new();
    let mut libraries_at: Vec<(String, PathBuf)> = Vec::new();
    let mut identity: Option<PathBuf> = None;
    let mut staging: Option<PathBuf> = None;
    let mut applications: Vec<PathBuf> = Vec::new();
    let mut launcher: Option<PathBuf> = None;
    let mut displayd: Option<PathBuf> = None;
    let mut proxyd: Option<PathBuf> = None;
    let mut dnsd: Option<PathBuf> = None;
    let mut gdbserver: Option<PathBuf> = None;
    let mut strace: Option<PathBuf> = None;
    let mut trusted_keys: Vec<(String, Vec<String>, PathBuf)> = Vec::new();
    let mut development_only = false;
    let mut document: Option<PathBuf> = None;
    let mut fleet_repository: Option<PathBuf> = None;
    let mut arch: Option<String> = None;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--device-mib" => {
                i += 1;
                let raw = argv.get(i).ok_or("--device-mib needs a value")?;
                device_mib = raw
                    .parse()
                    .map_err(|_| format!("--device-mib wants a number, got {raw:?}"))?;
            }
            "--splash" => {
                i += 1;
                let raw = argv.get(i).ok_or("--splash needs a path")?;
                splash = Some(PathBuf::from(raw));
            }
            "--bluetoothd" => {
                i += 1;
                let raw = argv.get(i).ok_or("--bluetoothd needs a path")?;
                bluetoothd = Some(PathBuf::from(raw));
            }
            "--supplicant" => {
                i += 1;
                let raw = argv.get(i).ok_or("--supplicant needs a path")?;
                supplicant = Some(PathBuf::from(raw));
            }
            "--wpa-cli" => {
                i += 1;
                let raw = argv.get(i).ok_or("--wpa-cli needs a path")?;
                wpa_cli = Some(PathBuf::from(raw));
            }
            "--udhcpc" => {
                i += 1;
                let raw = argv.get(i).ok_or("--udhcpc needs a path")?;
                udhcpc = Some(PathBuf::from(raw));
            }
            "--dhcp-hook" => {
                i += 1;
                let raw = argv.get(i).ok_or("--dhcp-hook needs a path")?;
                dhcp_hook = Some(PathBuf::from(raw));
            }
            "--kernel" => {
                i += 1;
                let raw = argv.get(i).ok_or("--kernel needs a path")?;
                kernel = Some(PathBuf::from(raw));
            }
            "--device-tree" => {
                i += 1;
                let raw = argv.get(i).ok_or("--device-tree needs a path")?;
                device_tree = Some(PathBuf::from(raw));
            }
            // **Repeatable, and it accumulates rather than replaces.** Every
            // other path option here names one file and the last wins; this one
            // names a set, and a second `--library` that discarded the first
            // would produce a card missing a library nobody could see was
            // asked for.
            // Two values, destination first, because that is the order the
            // sentence takes: put THIS file THERE.
            "--library-at" => {
                i += 1;
                let dest = argv.get(i).ok_or("--library-at needs a destination")?;
                i += 1;
                let src = argv.get(i).ok_or("--library-at needs a source path")?;
                if !dest.starts_with('/') {
                    return Err(format!("--library-at destination {dest:?} is not absolute"));
                }
                libraries_at.push((dest.clone(), PathBuf::from(src)));
            }
            "--library" => {
                i += 1;
                let raw = argv.get(i).ok_or("--library needs a path")?;
                libraries.push(PathBuf::from(raw));
            }
            "--loader" => {
                i += 1;
                let raw = argv.get(i).ok_or("--loader needs a path")?;
                loader = Some(PathBuf::from(raw));
            }
            "--wifi-firmware" => {
                i += 1;
                let raw = argv.get(i).ok_or("--wifi-firmware needs a directory")?;
                wifi_firmware = Some(PathBuf::from(raw));
            }
            "--femto" => {
                i += 1;
                let raw = argv.get(i).ok_or("--femto needs a path")?;
                femto = Some(PathBuf::from(raw));
            }
            "--nushell" => {
                i += 1;
                let raw = argv.get(i).ok_or("--nushell needs a path")?;
                nushell = Some(PathBuf::from(raw));
            }
            "--ca-bundle" => {
                i += 1;
                let raw = argv.get(i).ok_or("--ca-bundle needs a path")?;
                ca_bundle = Some(PathBuf::from(raw));
            }
            "--zoneinfo" => {
                i += 1;
                let raw = argv.get(i).ok_or("--zoneinfo needs a directory")?;
                zoneinfo = zone_database(std::path::Path::new(raw))?;
            }
            "--font" => {
                i += 1;
                let raw = argv.get(i).ok_or("--font needs a path")?;
                system_font = Some(PathBuf::from(raw));
            }
            "--mono-font" => {
                i += 1;
                let raw = argv.get(i).ok_or("--mono-font needs a path")?;
                mono_font = Some(PathBuf::from(raw));
            }
            "--icon-font" => {
                i += 1;
                let raw = argv.get(i).ok_or("--icon-font needs a path")?;
                icon_font = Some(PathBuf::from(raw));
            }
            "--gdbserver" => {
                i += 1;
                let raw = argv.get(i).ok_or("--gdbserver needs a path")?;
                gdbserver = Some(PathBuf::from(raw));
            }
            "--strace" => {
                i += 1;
                let raw = argv.get(i).ok_or("--strace needs a path")?;
                strace = Some(PathBuf::from(raw));
            }
            "--trusted-key" => {
                i += 1;
                let raw = argv
                    .get(i)
                    .ok_or("--trusted-key needs <label>:<kinds>:<path>")?;
                // Split from the left twice, so a Windows path keeping its
                // drive letter's colon still works.
                let (label, rest) = raw.split_once(':').ok_or_else(|| {
                    format!("--trusted-key wants <label>:<kinds>:<path>, got {raw:?}")
                })?;
                let (kinds, path) = rest.split_once(':').ok_or_else(|| {
                    format!("--trusted-key wants <label>:<kinds>:<path>, got {raw:?}")
                })?;
                if label.is_empty() {
                    return Err(format!("--trusted-key has an empty label: {raw:?}"));
                }
                let kinds: Vec<String> = kinds
                    .split(',')
                    .filter(|k| !k.is_empty())
                    .map(str::to_string)
                    .collect();
                if kinds.is_empty() {
                    return Err(format!(
                        "--trusted-key {label:?} names no kinds. A key trusted for nothing signs nothing; write the kinds it may sign, e.g. update,extension"
                    ));
                }
                trusted_keys.push((label.to_string(), kinds, PathBuf::from(path)));
            }
            "--no-trust" => development_only = true,
            "--identity" => {
                i += 1;
                let raw = argv.get(i).ok_or("--identity needs a directory")?;
                identity = Some(PathBuf::from(raw));
            }
            "--staging" => {
                i += 1;
                let raw = argv.get(i).ok_or("--staging needs a path")?;
                staging = Some(PathBuf::from(raw));
            }
            // **Repeatable and accumulating**, on `--library`'s terms — see
            // `Args::applications`.
            "--application" => {
                i += 1;
                let raw = argv.get(i).ok_or("--application needs a path to a .rvt")?;
                applications.push(PathBuf::from(raw));
            }
            "--launcher" => {
                i += 1;
                let raw = argv.get(i).ok_or("--launcher needs a path")?;
                launcher = Some(PathBuf::from(raw));
            }
            "--displayd" => {
                i += 1;
                let raw = argv.get(i).ok_or("--displayd needs a path")?;
                displayd = Some(PathBuf::from(raw));
            }
            "--proxyd" => {
                i += 1;
                let raw = argv.get(i).ok_or("--proxyd needs a path")?;
                proxyd = Some(PathBuf::from(raw));
            }
            "--dnsd" => {
                i += 1;
                let raw = argv.get(i).ok_or("--dnsd needs a path")?;
                dnsd = Some(PathBuf::from(raw));
            }
            "--document" => {
                i += 1;
                let raw = argv.get(i).ok_or("--document needs a path")?;
                document = Some(PathBuf::from(raw));
            }
            "--fleet-repository" => {
                i += 1;
                let raw = argv.get(i).ok_or("--fleet-repository needs a directory")?;
                fleet_repository = Some(PathBuf::from(raw));
            }
            "--arch" => {
                i += 1;
                let raw = argv.get(i).ok_or("--arch needs a token, e.g. aarch64")?;
                arch = Some(raw.clone());
            }
            "--default-slot" => {
                i += 1;
                default_slot = match argv.get(i).map(String::as_str) {
                    Some("a") | Some("A") => Slot::A,
                    Some("b") | Some("B") => Slot::B,
                    other => {
                        return Err(format!(
                            "--default-slot wants a or b, got {}",
                            other.unwrap_or("nothing")
                        ));
                    }
                };
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            _ => positional.push(&argv[i]),
        }
        i += 1;
    }

    let [
        image, init, supervisor, cli, netd, timed, mdnsd, git, http_backend,
        deploy, mcpd,
    ] =
        positional.as_slice()
    else {
        return Err(usage());
    };

    // **Required, and refused by name rather than defaulted.** Every binary on
    // this card is aarch64 today, and defaulting to that would be true for every
    // card this tool has built and wrong for the first one that is not — which a
    // machine then discovers by refusing every artifact. The card build knows the
    // target it compiled for and says so.
    let arch = arch.ok_or_else(|| {
        format!(
            "--arch is required: ADR-027 §3's token for the target these binaries were built \
             for ({}). The image records it at {} and the machine compares every artifact \
             against it (PLAN-Feature-Set-2026-09-12 §3).",
            rivet_core::artifact::ARCH_TOKENS.join(", "),
            system::ARCH_PATH
        )
    })?;

    Ok(Args {
        image: (*image).clone(),
        init: PathBuf::from(*init),
        supervisor: PathBuf::from(*supervisor),
        cli: PathBuf::from(*cli),
        netd: PathBuf::from(*netd),
        timed: PathBuf::from(*timed),
        mdnsd: PathBuf::from(*mdnsd),
        git: PathBuf::from(*git),
        http_backend: PathBuf::from(*http_backend),
        deploy: PathBuf::from(*deploy),
        mcpd: PathBuf::from(*mcpd),
        nushell,
        ca_bundle,
        system_font,
        mono_font,
        icon_font,
        zoneinfo,
        femto,
        splash,
        bluetoothd,
        supplicant,
        wpa_cli,
        udhcpc,
        dhcp_hook,
        kernel,
        device_tree,
        wifi_firmware,
        loader,
        libraries,
        libraries_at,
        gdbserver,
        strace,
        trusted_keys,
        development_only,
        document,
        fleet_repository,
        identity,
        staging,
        applications,
        launcher,
        displayd,
        proxyd,
        dnsd,
        arch,
        device_mib,
        default_slot,
    })
}

/// Subtrees of a distribution's zone database that a Rivet does **not** ship.
///
/// Two, and each is a decision rather than a size cut:
///
/// * **`posix/`** is a byte-for-byte duplicate of the top level. Verified
///   rather than assumed, on the tzdata 2026c tree in this project's own build
///   container on 2026-09-02: 598 files, sha256 compared pairwise against their
///   top-level counterparts, **0 differences**. Shipping it would be 1.5 MB of
///   the same bytes under a second set of names — `ADR-011`'s two-answers
///   failure in the trivial case, and half the database's size.
/// * **`right/`** counts leap seconds. A machine using one of those files
///   reports a `time_t` about 27 seconds away from every other machine's,
///   because POSIX time is *defined* not to count them — so timestamps compared
///   across two machines would silently disagree. That is a wrong answer, and
///   refusing to ship the files is the same judgement `rivet_core::etc` makes
///   about every file it leaves out.
const NOT_SHIPPED: &[&str] = &["posix", "right"];

/// Enumerate a zone database — `ADR-021` §9, 2026-09-02.
///
/// **Every file is READ and checked, not trusted by name.** A distribution's
/// `/usr/share/zoneinfo` carries `zone.tab`, `zone1970.tab`, `iso3166.tab`,
/// `tzdata.zi`, `leapseconds` and a `SECURITY` file beside the zones, and none
/// of them is a zone. `rivet_core::timezone::is_tzfile` is the same check musl
/// performs before it gives up and keeps UTC, so a file that reaches the image
/// is one musl would actually accept — which is `AGENTS.md` §5's *report what
/// the system said* applied to a build input.
///
/// **The names use `/` on every host.** `America/Chicago` is the identifier an
/// operator writes into the document and the path the projection composes; a
/// Windows build host walking this directory yields `America\Chicago`, and an
/// image built there would carry a zone nothing could ever look up.
///
/// **Sorted**, because `ADR-015` §4 signs the image and the same inputs have to
/// produce the same bytes. `system::tree` sorts too; doing it here as well
/// costs nothing and means this function's own output is a value rather than a
/// directory-order accident.
fn zone_database(root: &std::path::Path) -> Result<Vec<(String, PathBuf)>, String> {
    if !root.is_dir() {
        return Err(format!(
            "--zoneinfo {}: not a directory. It is a zone database such as \
             /usr/share/zoneinfo; without one this card can only keep UTC",
            root.display()
        ));
    }
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| format!("--zoneinfo: cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry
                .map_err(|e| format!("--zoneinfo: cannot read {}: {e}", dir.display()))?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                // A name that is not UTF-8 cannot be an IANA identifier, so it
                // cannot be a zone anybody could name in a document.
                continue;
            };
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            // `metadata` follows a symlink deliberately: some distributions
            // ship aliases as links, and a link copied as a link would dangle
            // on a machine that has no zoneinfo directory at that path. The
            // same reasoning `system::tree` records for the wireless firmware.
            let meta = std::fs::metadata(entry.path())
                .map_err(|e| format!("--zoneinfo: cannot stat {}: {e}", entry.path().display()))?;
            if meta.is_dir() {
                if prefix.is_empty() && NOT_SHIPPED.contains(&name.as_str()) {
                    continue;
                }
                stack.push((entry.path(), relative));
                continue;
            }
            let bytes = std::fs::read(entry.path())
                .map_err(|e| format!("--zoneinfo: cannot read {}: {e}", entry.path().display()))?;
            if !rivet_core::timezone::is_tzfile(&bytes) {
                continue;
            }
            // A name the schema would refuse is a zone nothing could ask for,
            // so it is left out rather than shipped as an unreachable file.
            if rivet_core::timezone::Timezone::new(&relative).is_none() {
                continue;
            }
            found.push((relative, entry.path()));
        }
    }
    if found.is_empty() {
        return Err(format!(
            "--zoneinfo {}: no file in it begins with TZif, so it is not a zone database",
            root.display()
        ));
    }
    found.sort();
    Ok(found)
}

/// Read one input file, saying which one when it is missing.
///
/// A bare `No such file or directory` during an image build is the least
/// helpful thing a tool can say — `AGENTS.md` §3, refuse and name what is
/// wrong.
fn read_input(dir: &str, name: &str, how_to_get_it: &str) -> Result<Vec<u8>, String> {
    let path = PathBuf::from(dir).join(name);
    std::fs::read(&path)
        .map_err(|e| format!("cannot read {}: {e}\n  {how_to_get_it}", path.display()))
}

fn run() -> Result<String, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv)?;

    // --- inputs ---------------------------------------------------------
    // **The kernel and its device tree are ONE decision.** A tree from one
    // build beside an image from another is the pairing nobody should be able
    // to make by forgetting a flag, so half a pair is refused rather than
    // silently completed from the fallback directory.
    if args.kernel.is_some() != args.device_tree.is_some() {
        return Err(
            "--kernel and --device-tree go together. A device tree from one kernel beside an \
             image from another boots, and then disagrees with itself about the hardware."
                .to_string(),
        );
    }
    let kernel = match &args.kernel {
        Some(path) => std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        None => read_input(
            KERNEL_DIR,
            boot::KERNEL_FILE,
            "run tools/qemu/fetch-kernel.sh user@your-pi, or pass --kernel",
        )?,
    };
    let device_tree = match &args.device_tree {
        Some(path) => std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        None => read_input(
            KERNEL_DIR,
            boot::DEVICE_TREE_FILE,
            "run tools/qemu/fetch-kernel.sh user@your-pi, or pass --device-tree",
        )?,
    };
    let mut firmware = Vec::new();
    for name in boot::REQUIRED_FIRMWARE {
        firmware.push(boot::BootFile {
            name: (*name).to_string(),
            bytes: read_input(FIRMWARE_DIR, name, "run tools/fetch-firmware.sh")?,
        });
    }

    // --- the /system tree, identical in both slots ------------------------
    //
    // Both slots get the same image on a fresh card. `ADR-018` §5.2 writes the
    // *inactive* slot during an update, so A and B diverge only after one.
    // Read each trusted key off disk and build `ADR-028` §5.5's store.
    //
    // The encoding rule is `rivet_core::package::key_material`'s and is not
    // restated here: 32 raw bytes or 64 hex characters, nothing padded and
    // nothing truncated. Until 2026-08-24 this accepted **only** raw bytes
    // while `cargo rivet package` accepted both, so a hex key one tool signed
    // with was refused by the other with a message about its length. Two
    // answers to *what does a key file look like* is `ADR-011` §3.
    let mut entries = Vec::new();
    for (label, kinds, path) in &args.trusted_keys {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("cannot read the trusted key at {}: {e}", path.display()))?;
        let raw = rivet_core::package::key_material::parse(&bytes)
            .map_err(|e| format!("the trusted key {}: {e}", path.display()))?;
        let mut parsed = Vec::new();
        for k in kinds {
            parsed.push(
                rivet_core::package::Kind::parse(k)
                    .map_err(|e| format!("--trusted-key {label}: {k:?} is not a kind: {e}"))?,
            );
        }
        entries.push(rivet_core::package::trust_store::Entry {
            key: rivet_core::package::port::PublicKey(raw),
            label: label.clone(),
            may_sign: rivet_core::package::port::MaySign::these(&parsed),
        });
    }
    let trust = rivet_core::package::trust_store::TrustStore::new(entries);

    // --- selection: the role's required payloads, resolved by digest --------
    //
    // **`PLAN-Feature-Set-2026-09-12` §4, `ADR-050` §5.1.** `--fleet-repository`
    // turns "which products this role needs" — already decided, and already
    // read back off the composed document by `Document::carried_services` —
    // into paths `application::verify` below can read exactly as it reads an
    // explicit `--application PATH`. Nothing is trusted yet: resolution only
    // finds bytes, and the verification loop right after this is what decides
    // whether this image will actually carry them.
    let mut application_paths = args.applications.clone();
    if let Some(fleet_root) = &args.fleet_repository {
        let Some(document_path) = &args.document else {
            return Err(
                "--fleet-repository was given with no --document, so there is no composed \
                 document to read a role's required services off. Pass --document, or drop \
                 --fleet-repository and carry payloads with --application instead."
                    .to_string(),
            );
        };
        // Read a second time, separately from the copy this function reads
        // further down to write onto both boot partitions: that later read
        // stays deliberately unparsed (see `--document`'s own usage text),
        // and threading a parsed value backward past it would couple the two
        // for no reader's benefit. The file is a few kilobytes; a build reads
        // it twice rather than the crate growing a second `document` binding.
        let text = std::fs::read_to_string(document_path).map_err(|e| {
            format!("cannot read the provisioning document at {}: {e}", document_path.display())
        })?;
        let parsed = rivet_config::Document::parse(&text).map_err(|e| {
            format!(
                "--fleet-repository needs {} to parse so its role's required services can be \
                 read: {e:?}",
                document_path.display()
            )
        })?;
        for service in parsed.carried_services() {
            let product = service.name().as_str();
            // `.rulings/` R-159 R20: the document (`crate::carry`) may pin
            // which version this product must be resolved at; `None` leaves
            // `resolve` to its own unpinned rule.
            let resolved = fleet_payloads::resolve(fleet_root, product, parsed.carried_version(product))?;
            application_paths.push(resolved);
        }
    }

    // --- the carried payloads, VERIFIED before a partition is written --------
    //
    // **`ADR-050` §4, and the placement is the decision.** A payload signed by
    // a key this image's trust store will not accept is a card that boots to a
    // machine with no fleet server, and the only account of it is a journal
    // record on a headless board. `crate::application` runs the verifier
    // `cmd/rivet-deploy` runs, against the trust store built two statements
    // above and about to be serialised onto `/system` — so what is asserted
    // here is not *this signature is valid* but **this card will install this**.
    //
    // The whole set is verified before any of it is placed. A build that
    // refused on the fourth payload having already written three would leave
    // the operator to work out which cards are which.
    let mut carried: Vec<application::Carried> = Vec::new();
    for path in &application_paths {
        carried.push(application::verify(path, &trust)?);
    }
    // **Two payloads for one product is a card that carries one of them**, and
    // FAT decides which. The refusal names both products rather than the file
    // names, because the name that collides is the one out of the signed
    // header and not the one the operator typed — and it is case-insensitive
    // because the boot partition is FAT, where `app-Foo.rvt` and `app-foo.rvt`
    // are one file. `rivet_core::carried`'s
    // `two_products_differing_only_in_case_are_two_products_here` is the other
    // half of this guard.
    for (i, one) in carried.iter().enumerate() {
        if let Some(other) = carried
            .iter()
            .skip(i.saturating_add(1))
            .find(|o| o.product.eq_ignore_ascii_case(&one.product))
        {
            return Err(format!(
                "two payloads claim the product {:?} and {:?}, and the boot partition is FAT, \
                 where those are one file name. One of them would silently not be on the card. \
                 The name comes from each payload's SIGNED header (ADR-015 §7), so pass one \
                 of them or rebuild the other under its own name.",
                one.product, other.product
            ));
        }
    }

    if trust.is_empty() && args.development_only {
        eprintln!(
            "rivet-image: WARNING building a DEVELOPMENT-ONLY image: it trusts no signing key, so it can verify no update and cannot be updated safely (ADR-028 §5.5.5)."
        );
    }

    let nodes = system::tree(&system::Inputs {
        init: args.init.clone(),
        supervisor: args.supervisor.clone(),
        cli: args.cli.clone(),
        netd: args.netd.clone(),
        timed: args.timed.clone(),
        mdnsd: args.mdnsd.clone(),
        git: args.git.clone(),
        http_backend: args.http_backend.clone(),
        deploy: args.deploy.clone(),
        nushell: args.nushell.clone(),
        ca_bundle: args.ca_bundle.clone(),
        system_font: args.system_font.clone(),
        mono_font: args.mono_font.clone(),
        icon_font: args.icon_font.clone(),
        zoneinfo: args.zoneinfo.clone(),
        femto: args.femto.clone(),
        splash: args.splash.clone(),
        bluetoothd: args.bluetoothd.clone(),
        launcher: args.launcher.clone(),
        displayd: args.displayd.clone(),
        proxyd: args.proxyd.clone(),
        dnsd: args.dnsd.clone(),
        supplicant: args.supplicant.clone(),
        wpa_cli: args.wpa_cli.clone(),
        udhcpc: args.udhcpc.clone(),
        dhcp_hook: args.dhcp_hook.clone(),
        wifi_firmware: args.wifi_firmware.clone(),
        loader: args.loader.clone(),
        libraries: args.libraries.clone(),
        libraries_at: args.libraries_at.clone(),
        mcpd: args.mcpd.clone(),
        gdbserver: args.gdbserver.clone(),
        strace: args.strace.clone(),
        trust,
        arch: args.arch.clone(),
        development_only: args.development_only,
        login_shells: system::LOGIN_SHELLS,
    })
    .map_err(|e| format!("{e}"))?;
    let system_entries = system_entries(&nodes)?;

    // --- assemble, keyed by partition number ------------------------------
    let mut trees: BTreeMap<u32, disk::Tree> = BTreeMap::new();
    // The operator's provisioning document, read once and written into BOTH
    // boot partitions — a card is provisioned, not a slot, and an update that
    // swapped slots must not swap away the machine's address and keys.
    let document = match &args.document {
        Some(path) => Some(std::fs::read_to_string(path).map_err(|e| {
            format!(
                "cannot read the provisioning document at {}: {e}",
                path.display()
            )
        })?),
        None => None,
    };

    // **All three or none, checked HERE.** `cmd/rivet-initd` refuses a seed with
    // no certificate too, but it does so on a BOARD -- the most expensive place
    // to learn that a card was written wrong.
    let identity = match &args.identity {
        Some(dir) => {
            let read = |name: &str| -> Result<Vec<u8>, String> {
                let path = dir.join(name);
                std::fs::read(&path).map_err(|e| {
                    format!(
                        "cannot read {} for --identity: {e}. `cargo rivet identity new` writes \
                         device.seed and device.crt; the peers file is the operator's, one hex \
                         public key per line",
                        path.display()
                    )
                })
            };
            let seed = read(boot::IDENTITY_SEED_FILE)?;
            if seed.len() != 32 {
                return Err(format!(
                    "{} is {} bytes and an Ed25519 seed is exactly 32. The file holds RAW bytes, not hex and not base64",
                    dir.join(boot::IDENTITY_SEED_FILE).display(),
                    seed.len()
                ));
            }
            Some(boot::Identity {
                seed,
                certificate: read(boot::IDENTITY_CERTIFICATE_FILE)?,
                peers: read(boot::IDENTITY_PEERS_FILE)?,
            })
        }
        None => None,
    };

    // **The staging bundle, PARSED here and not merely copied** — `ADR-049`,
    // and it is the one place in this crate that reads what it carries.
    //
    // `document` and `identity` above are deliberately opaque bytes: this crate
    // does not depend on `rivet-config`, so it cannot say whether a provisioning
    // document parses, and the refusal for a bad one comes from the board. That
    // seam is affordable there because a machine with an unreadable document
    // still boots and can be reached.
    //
    // It is not affordable here. A bundle PID 1 refuses leaves the material on
    // the card and the application without its credentials, and the report is a
    // line on a console nobody is watching on a headless board. `rivet-core` is
    // already a dependency of this crate, so the parser costs nothing and the
    // refusal lands on the workstation, before a card is written.
    let staged = match &args.staging {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                format!(
                    "cannot read the staging bundle at {}: {e}. `rivet-provision` writes one when \
                     it composes a machine's document",
                    path.display()
                )
            })?;
            let items = rivet_core::staging::parse(&text)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if items.is_empty() {
                return Err(format!(
                    "{} stages nothing. An empty bundle is a card that carries a private key's \
                     worth of risk and delivers no credential; omit --staging instead",
                    path.display()
                ));
            }
            Some(text)
        }
        None => None,
    };

    for slot in [Slot::A, Slot::B] {
        let files = boot::contents(
            &boot::BootPartitionSpec {
                slot,
                default_slot: args.default_slot,
            },
            &boot::BootInputs {
                kernel: kernel.clone(),
                device_tree: device_tree.clone(),
                firmware: firmware.clone(),
                document: document.clone(),
                identity: identity.clone(),
                staged: staged.clone(),
                // **Both boot partitions, like the document.** A card is
                // provisioned rather than a slot, and an update that swapped
                // slots must not swap away the applications the card was built
                // to carry. PID 1 reads whichever partition it booted.
                applications: carried
                    .iter()
                    .map(|c| boot::BootFile::new(c.card_file(), c.bytes.clone()))
                    .collect(),
            },
        )
        .map_err(|e| format!("{e}"))?;

        let entries = files
            .into_iter()
            .map(|f| disk::Entry::File {
                path: f.name,
                mode: 0o644,
                contents: f.bytes,
            })
            .collect();
        trees.insert(
            slot.boot_partition(),
            disk::Tree::new(entries).map_err(|e| format!("{e}"))?,
        );
        trees.insert(
            slot.system_partition(),
            disk::Tree::new(system_entries.clone()).map_err(|e| format!("{e}"))?,
        );
    }
    // `/state` ships empty, and nothing creates a tree inside it.
    //
    // **Corrected 2026-08-23.** This line used to say "PID 1 creates its tree as
    // root on first boot". PID 1 now mounts `p5` (`ADR-017` §3.1.7) and
    // deliberately creates nothing in it: `mkdir` is idempotent, so whichever
    // component first writes to `/state/config`, `/state/home` or `/state/ext`
    // makes its own directory, and a list of paths inside PID 1 is exactly the
    // creep §3.1.5 exists to resist. The old sentence was a plan and was never
    // what was built.

    let mut host = disk::CommandHost::new();
    let report = disk::build(
        &mut host,
        &disk::BuildRequest {
            image_path: &args.image,
            device_mib: args.device_mib,
            trees,
        },
    )
    .map_err(|e| format!("{e}"))?;

    Ok(format!("{}{}", render(&report), render_carried(&carried)))
}

/// What this card carries, and the sentence that keeps the claim honest.
///
/// **Empty for a card that carries nothing**, which is every card built before
/// 2026-09-07 and every headless one after it — `AGENTS.md` §3's *say what is
/// there*, not a line saying nothing is.
///
/// The last paragraph is the point. `ADR-050` §4 accepts that a carried
/// payload can fail to install, and §10 rules that the machine comes up without
/// it and says so rather than refusing to converge. So the build must not let
/// anybody read this list as *these applications will be running*: what has
/// been proved here is that the bytes verify against this card's own trust
/// store, and the machine is where the install actually happens.
fn render_carried(carried: &[application::Carried]) -> String {
    if carried.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n--- applications this card carries (ADR-050 §4) ---\n");
    for one in carried {
        out.push_str(&format!(
            "  {:<24} {:<10} {:<12} {:>10} bytes  -> {}\n",
            one.product,
            one.kind,
            one.version,
            one.bytes.len(),
            one.card_file(),
        ));
    }
    out.push_str(
        "\nEach VERIFIED against this image's own trust store, which is the check a board\n\
         would make. That is not the same as installed: PID 1 lands them in /state/carried\n\
         and rivet-deploy installs each one on a convergence pass. A payload that fails\n\
         there leaves the machine up and reachable, with the reason in `rivet get log`\n\
         (ADR-050 §10) — it does NOT stop the machine converging.\n",
    );
    out
}

/// Render the report for a human.
///
/// `ADR-006` §4 makes the structured value primary and text a *view* of it,
/// which is why [`disk::BuildReport`] has no `Display` of its own: rendering is
/// a presentation concern and belongs at the edge, in the composition root.
fn render(report: &disk::BuildReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} — {} MiB\n\n",
        report.image_path, report.device_mib
    ));
    out.push_str("  # label      fs      offset        size    reproducible\n");
    for p in &report.partitions {
        out.push_str(&format!(
            "  {} {:<10} {:<6} {:>11}  {:>10}    {}\n",
            p.number,
            p.label,
            p.fstype.as_str(),
            p.offset_bytes,
            p.len_bytes,
            match p.irreproducible_because {
                None => "yes".to_string(),
                Some(why) => format!("NO — {why}"),
            }
        ));
    }
    out.push('\n');

    if report.fully_reproducible() {
        out.push_str("Byte-reproducible: two builds of the same inputs are identical.\n");
    } else {
        out.push_str(
            "NOT byte-reproducible. `ADR-015` §4 signs these artifacts, and a signature\n\
             over something nobody can rebuild is worth less than it looks.\n",
        );
    }

    // The default device size is the layout's minimum, which makes /state
    // small on purpose (see default_device_mib). That is only safe because
    // ADR-018 §10 grows it on first boot, and §10 is not implemented yet — so
    // an operator who writes this card today gets a /state that boots and then
    // fills. AGENTS.md §3: say it, every time, rather than letting a full
    // filesystem be the notification.
    if report.device_mib <= layout::minimum_device_mib() {
        let state_mib = report
            .partitions
            .iter()
            .find(|p| p.number == layout::STATE_PARTITION)
            .map_or(0, |p| p.len_bytes / (1024 * 1024));
        out.push_str(&format!(
            "\n/state is {state_mib} MiB — this card is at the layout's minimum ({} MiB).\n\
             `ADR-018` §10.3 grows it to fill the device, and since 2026-08-25 the\n\
             supervisor CALLS it: a one-shot before it converges any service.\n\
             \n\
             IT HAS RUN ON A CARD, ONCE, AND IT WORKED. Read off the Pi 4 on\n\
             2026-08-25: /state was 5 700 977 664 bytes on a 7.42 GiB card written\n\
             from a 2 GiB image — so the GPT rewrite and the ext4 resize both\n\
             executed on real media. `PLAN.md` §13.4's 232 MiB is the reading from\n\
             BEFORE that, and this sentence said NEVER RUN until the same day.\n\
             \n\
             What is still unproven is the grow on a card of a different size, and\n\
             the failure path — nothing has watched it refuse. `rivet get storage`\n\
             on the first boot is still how you read the answer.\n\
             \n\
             IF IT DOES NOT GROW, THIS CARD CANNOT RECEIVE A SYSTEM UPDATE.\n\
             `rivet apply install` names a payload already on the machine, by its\n\
             digest (ADR-052 §7) — so the whole payload still has to fit on /state\n\
             before it can be installed, and a system update for this layout is\n\
             about 683 MB against a /state of {state_mib} MiB. Measured on a board\n\
             2026-08-25, `PLAN.md` §13.4.\n\
             \n\
             Pass --device-mib matching the physical card and none of the above\n\
             applies — which is still the certain answer where the grow is the\n\
             probable one.\n",
            layout::minimum_device_mib()
        ));
    }

    out.push_str(
        "\nThis is an image file. It has not been written to anything, and no Raspberry\n\
         Pi has ever read one — `ADR-026` §4's right-hand column is untouched. Check the\n\
         device path yourself before writing it.\n",
    );
    out
}

/// Translate `system`'s nodes into `disk`'s entries.
///
/// The two modules were written against each other's *shape* rather than each
/// other's types, deliberately — so this is where the shapes meet, and it is
/// the only place that knows both.
///
/// `system` paths are absolute (where they land on the booted machine) and
/// `disk` paths are relative to a filesystem's root, so the leading `/` comes
/// off here. `uid`/`gid` are dropped because `disk` forces `0:0` through
/// `--all-root` and a tarball — the assertion below is what stops that being a
/// silent mismatch if `system` ever carries a non-root uid.
fn system_entries(nodes: &[system::Node]) -> Result<Vec<disk::Entry>, String> {
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        if n.uid != system::ROOT_UID || n.gid != system::ROOT_GID {
            return Err(format!(
                "{} wants uid {}:{} but an unprivileged build can only make root-owned \
                 entries (disk.rs forces 0:0 via --all-root and a tarball). This needs \
                 a decision, not a workaround.",
                n.path, n.uid, n.gid
            ));
        }
        let path = n.path.trim_start_matches('/').to_string();
        if path.is_empty() {
            continue; // `/` itself; the filesystem has one already.
        }
        out.push(match &n.content {
            system::Content::Directory => disk::Entry::Dir { path, mode: n.mode },
            system::Content::CopiedFrom(src) => disk::Entry::File {
                path,
                mode: n.mode,
                contents: std::fs::read(src)
                    .map_err(|e| format!("cannot read {}: {e}", src.display()))?,
            },
            // The `/etc` files. No I/O: they were rendered from
            // `rivet_core::etc`'s definitions, which is what keeps two builds
            // of the same inputs byte-identical without a scratch file whose
            // mtime would have to be pinned as well.
            system::Content::Generated(bytes) => disk::Entry::File {
                path,
                mode: n.mode,
                contents: bytes.clone(),
            },
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- the zone database, ADR-021 §9 -------------------------------------

    /// A directory shaped like a distribution's `/usr/share/zoneinfo`: real
    /// zones, the tab files that sit beside them, and the two subtrees Rivet
    /// refuses. Written into a temporary directory rather than committed —
    /// `AGENTS.md` §8.3, and a TZif is binary.
    fn a_zone_database() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rivet-zoneinfo-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mut tzif = Vec::from(*rivet_core::timezone::TZIF_MAGIC);
        tzif.extend(std::iter::repeat_n(0u8, 60));
        for (relative, bytes) in [
            ("UTC", tzif.clone()),
            ("America/Chicago", tzif.clone()),
            ("America/Argentina/Buenos_Aires", tzif.clone()),
            // Beside the zones, and not one: every distribution ships these.
            ("zone1970.tab", b"# a table, not a zone\n".to_vec()),
            ("tzdata.zi", b"# source, not a zone\n".to_vec()),
            // The two subtrees that are refused by name.
            ("posix/America/Chicago", tzif.clone()),
            ("right/America/Chicago", tzif.clone()),
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(&path, &bytes).expect("write");
        }
        root
    }

    /// **Every file is read, not trusted by name.** The tab files are the
    /// reason: they sit in the same directory as the zones and are not zones,
    /// and a build that shipped them would put files on the card that nothing
    /// can ever look up.
    #[test]
    fn only_actual_tzfiles_reach_the_image() {
        let root = a_zone_database();
        let found = zone_database(&root).expect("a database");
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["America/Argentina/Buenos_Aires", "America/Chicago", "UTC"],
            "sorted, and only the tzfiles"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `posix/` duplicates the top level byte for byte and `right/` counts leap
    /// seconds — see [`NOT_SHIPPED`] for what each costs.
    #[test]
    fn the_two_refused_subtrees_do_not_reach_the_image() {
        let root = a_zone_database();
        let found = zone_database(&root).expect("a database");
        for (name, _) in &found {
            assert!(
                !name.starts_with("posix/") && !name.starts_with("right/"),
                "{name} reached the image"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A directory that is not a database is refused rather than producing an
    /// image with an empty `/system/share/zoneinfo` — which would look like a
    /// card that could keep a timezone and could not.
    #[test]
    fn a_directory_with_no_zones_in_it_is_refused_by_name() {
        let root = std::env::temp_dir().join(format!("rivet-not-zoneinfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("readme"), b"nothing here is a zone").expect("write");
        let refusal = zone_database(&root).expect_err("not a database");
        assert!(refusal.contains("TZif"), "{refusal}");
        assert!(
            zone_database(&root.join("does-not-exist"))
                .expect_err("not a directory")
                .contains("not a directory")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **Inverted 2026-08-23, and the inversion is the point.**
    ///
    /// This asserted `>`: the default had to be strictly larger than the
    /// minimum, so `/state` got room on a fresh card. The default is now the
    /// minimum exactly, because sizing the image to the card is what made
    /// writing one cost twenty minutes to deliver 67 MB of content, and
    /// `ADR-018` §10 moves the growing of `/state` to first boot where the
    /// device's real size is knowable.
    ///
    /// Equality is therefore correct and *smaller* is still a defect, so the
    /// assertion stays — flipped, not deleted. `plan` refuses anything under
    /// the minimum, and a default it refused would be a tool that cannot run
    /// with no arguments at all.
    #[test]
    fn the_default_is_exactly_the_minimum_and_never_below_it() {
        assert_eq!(
            default_device_mib(),
            layout::minimum_device_mib(),
            "the default card size is deliberately the minimum; see ADR-018 §10"
        );
    }

    /// The warning in [`render`] is load-bearing while `ADR-018` §10 is unbuilt:
    /// it is the only thing standing between the minimum default and an
    /// operator discovering a small `/state` when it fills.
    #[test]
    fn a_minimum_sized_card_is_warned_about_rather_than_shipped_quietly() {
        let Ok(partitions) = disk::plan(default_device_mib()) else {
            return;
        };
        let report = disk::BuildReport {
            image_path: "/out/i.img".to_string(),
            device_mib: default_device_mib(),
            partitions: partitions
                .iter()
                .map(|p| disk::PartitionReport {
                    number: p.spec.number,
                    label: p.spec.label,
                    fstype: p.spec.fstype,
                    first_lba: p.first_lba,
                    last_lba: p.first_lba + p.sectors() - 1,
                    offset_bytes: p.first_lba * 512,
                    len_bytes: p.sectors() * 512,
                    used_bytes: None,
                    irreproducible_because: None,
                })
                .collect(),
        };
        let text = render(&report);
        assert!(text.contains("ADR-018` §10"), "warning does not cite §10");
        // **The claim to pin is not "§10 is unbuilt".** That was the assertion
        // until 2026-08-25 and it went stale the hour the supervisor started
        // calling the grow — a test asserting a *state of the project* rather
        // than a *property of the output*, which is the shape `AGENTS.md` §7
        // keeps catching.
        //
        // What must survive every future state of §10 is that a card built at
        // the minimum tells its reader the two things that can cost them a
        // drive to the board: that nothing has proven the grow on real media,
        // and that a `/state` which does not grow cannot take an update.
        // **Was `contains("NEVER RUN ON A CARD")` until 2026-08-25**, when a
        // board was read and the claim turned out to be false — `AGENTS.md` §7
        // in the artifact that quotes §7 at itself six lines up. What the
        // warning owes a reader is not a *particular* sentence but an honest
        // one, so this now asserts that the two remaining unproven things are
        // named rather than that a specific stale phrase survives.
        assert!(
            text.contains("unproven") && text.contains("different size"),
            "the warning must say what about the grow is still NOT proven:\n{text}"
        );
        assert!(
            text.contains("CANNOT RECEIVE A SYSTEM UPDATE"),
            "the warning must name the consequence, not just the size:\n{text}"
        );
        assert!(
            text.contains("--device-mib"),
            "a warning with no remedy in it is a complaint:\n{text}"
        );
    }

    #[test]
    fn the_defaults_fit_the_layout_they_are_defaults_for() {
        assert!(
            default_device_mib() >= layout::minimum_device_mib(),
            "the default card size must not be one `plan` would refuse"
        );
    }

    /// The positionals, spelled once.
    ///
    /// **There were ten copies of this array until 2026-08-26**, and adding
    /// `rivet-gitd` and `git` broke four tests that are not about positionals
    /// at all — the slot flag, the nushell flag, the debug flags. A test that
    /// fails for a reason it is not about is a test that will be edited into
    /// passing rather than read.
    ///
    /// **It earned itself back on 2026-08-30**, when `rivet-gitd` was removed
    /// again and the same five tests failed for the same reason. One edit here
    /// fixed all of them, which is the whole argument for spelling it once.
    const POSITIONALS: [&str; 11] = [
        "a.img", "i", "s", "c", "n", "t", "m", "g", "hb", "dp", "mc",
    ];

    /// The one flag every build must pass, spelled once for `POSITIONALS`'
    /// reason: a test about the slot flag must not fail because `--arch`
    /// became required.
    const REQUIRED_FLAGS: [&str; 2] = ["--arch", "aarch64"];

    /// Every positional and every required flag, then whatever flags the test
    /// is actually about.
    fn argv(flags: &[&str]) -> Vec<String> {
        POSITIONALS
            .iter()
            .chain(REQUIRED_FLAGS.iter())
            .chain(flags.iter())
            .map(|a| (*a).to_string())
            .collect()
    }

    /// **`--arch` is required, reaches the field, and its absence names what it
    /// is for** — `PLAN-Feature-Set-2026-09-12` §3. A card without the record
    /// would boot and refuse every artifact, so this is a build-time refusal.
    #[test]
    fn the_architecture_is_required_and_reaches_the_field() {
        let without: Vec<String> = POSITIONALS.iter().map(|a| (*a).to_string()).collect();
        let Err(why) = parse_args(&without) else {
            panic!("a build with no --arch was accepted");
        };
        assert!(why.contains("--arch"), "{why}");
        assert!(why.contains("ADR-027"), "{why}");
        assert!(matches!(parse_args(&argv(&[])), Ok(a) if a.arch == "aarch64"));
        let mut dangling: Vec<String> = POSITIONALS.iter().map(|a| (*a).to_string()).collect();
        dangling.push("--arch".to_string());
        assert!(parse_args(&dangling).is_err(), "--arch with no value was accepted");
    }

    #[test]
    fn a_missing_argument_is_refused_with_the_usage_rather_than_a_panic() {
        // Every arity below the full one, so that adding a positional and
        // forgetting to require it fails here. The count has moved most times
        // this crate has been touched — up as each binary was added, and down
        // twice as one was removed — and each new arity is exactly the one this
        // test did not previously cover.
        //
        // **The count is read off the array rather than written in the message**,
        // because a number in a sentence is `AGENTS.md` §7's staleness and this
        // one has now gone stale four times.
        //
        // **And it now reads `POSITIONALS` rather than a second copy of it.**
        // There was a duplicate here — the very thing that constant was written
        // to abolish — and it survived the 2026-08-26 consolidation unnoticed,
        // then failed on its own during the 2026-08-30 removal. A copy that is
        // not used by `argv()` is a copy nobody thinks to update.
        let all = POSITIONALS;
        for n in 0..all.len() {
            let argv: Vec<String> = all[..n].iter().map(|a| (*a).to_string()).collect();
            assert!(
                parse_args(&argv).is_err(),
                "{n} positional argument(s) was accepted; the image needs {}",
                all.len()
            );
        }
        assert!(
            parse_args(&argv(&[])).is_ok(),
            "{} positional arguments must be accepted",
            all.len()
        );
    }

    /// The positionals are read in the order the usage prints them. A
    /// transposition here would build a card whose PID 1 is the CLI, which
    /// panics the kernel — and would do it silently, because every one of them
    /// is an executable file that copies without complaint.
    ///
    /// **Each one added makes that worse rather than merely longer**, and `git`
    /// is the sharp case: it is the one input that is not a Rivet binary, so a
    /// transposition putting it at `/system/bin/rivet` would produce a card
    /// whose *login shell is `git`* — and `check_login_shells` would accept it,
    /// because all that guard can see is that something executable is at the
    /// path.
    #[test]
    fn the_positionals_land_in_the_order_the_usage_states() {
        let argv = [
            "out.img", "init", "sup", "cli", "netd", "timed", "mdnsd",
            "git", "http-backend", "deploy", "mcpd", "--arch", "aarch64",
        ]
        .map(String::from);
        let Ok(args) = parse_args(&argv) else {
            return;
        };
        assert_eq!(args.image, "out.img");
        assert_eq!(args.init, PathBuf::from("init"));
        assert_eq!(args.supervisor, PathBuf::from("sup"));
        assert_eq!(args.cli, PathBuf::from("cli"));
        assert_eq!(args.netd, PathBuf::from("netd"));
        assert_eq!(args.timed, PathBuf::from("timed"));
        assert_eq!(args.mdnsd, PathBuf::from("mdnsd"));
        assert_eq!(args.git, PathBuf::from("git"));
        assert_eq!(args.http_backend, PathBuf::from("http-backend"));
        assert_eq!(args.deploy, PathBuf::from("deploy"));
        assert_eq!(args.mcpd, PathBuf::from("mcpd"));

        let usage = usage();
        let order = [
            "OUT.img",
            "INIT",
            "SUPERVISOR",
            "CLI",
            "NETD",
            "TIMED",
            "MDNSD",
            "GIT",
            "HTTP-BACKEND",
            "DEPLOY",
            "MCPD",
        ];
        let mut at = 0;
        for token in order {
            let found = usage.get(at..).and_then(|rest| rest.find(token));
            assert!(
                found.is_some(),
                "the usage does not name {token} after the argument before it"
            );
            at += found.unwrap_or(0) + token.len();
        }
    }

    /// A stray `--wat` must not be silently taken as the image path.
    #[test]
    fn an_unknown_option_is_refused_rather_than_treated_as_a_filename() {
        let argv = argv(&["--wat"]);
        assert!(
            parse_args(&argv).is_err(),
            "--wat was accepted; an unknown flag must not fall through to positionals"
        );
    }

    #[test]
    fn the_slot_flag_takes_a_or_b_and_refuses_anything_else() {
        // `matches!` rather than `unwrap`/`expect`/`panic!`, all of which the
        // workspace lints warn on — in tests too, deliberately, so the habit
        // never reaches the binaries where a panic is a kernel panic.
        let ok = argv(&["--default-slot", "b"]);
        assert!(matches!(parse_args(&ok), Ok(a) if a.default_slot == Slot::B));

        let upper =
            argv(&["--default-slot", "B"]);
        assert!(matches!(parse_args(&upper), Ok(a) if a.default_slot == Slot::B));

        let bad = argv(&["--default-slot", "c"]);
        assert!(parse_args(&bad).is_err());

        let missing = argv(&["--default-slot"]);
        assert!(parse_args(&missing).is_err());
    }

    /// A fresh card defaults to slot A, and both slots are written.
    #[test]
    fn the_default_slot_is_a_when_nothing_says_otherwise() {
        let argv = argv(&[]);
        assert!(matches!(parse_args(&argv), Ok(a) if a.default_slot == Slot::A));
    }

    /// `--nushell` is the one optional input, and its absence is the **default
    /// card** rather than an error.
    ///
    /// The failure this guards against is subtle and would be silent: an
    /// option that parsed but was never stored produces a card with no `nu`,
    /// a working `/etc/passwd` naming the CLI, and an operator who typed the
    /// flag and got nothing. Nothing else in the build refers to the value, so
    /// there is no second symptom — which is why every optional input is
    /// asserted in the source-reading test above.
    #[test]
    fn nushell_is_optional_and_the_path_reaches_the_field() {
        let without = argv(&[]);
        assert!(
            matches!(parse_args(&without), Ok(a) if a.nushell.is_none()),
            "a build without --nushell must succeed: it is every card built before 2026-08-24"
        );

        let with = argv(&["--nushell", "/b/nu"]);
        assert!(
            matches!(parse_args(&with), Ok(a) if a.nushell == Some(PathBuf::from("/b/nu"))),
            "--nushell parsed and the path did not reach the field"
        );

        let missing = argv(&["--nushell"]);
        assert!(
            parse_args(&missing).is_err(),
            "--nushell with no value must be refused, not treated as absent"
        );

        // And it must not be swallowed as a positional, which would make the
        // seven-positional match fail in a way that reads as a usage error.
        assert!(
            matches!(parse_args(&with), Ok(a) if a.image == "a.img"),
            "--nushell's value was counted as a positional"
        );
    }

    /// `--gdbserver` and `--strace` follow `--nushell` exactly, and each is
    /// asserted separately because they are separate match arms — a test that
    /// only tried one would pass with the other's arm missing entirely.
    ///
    /// The specific failure guarded is the one `nushell_is_optional_…` names and
    /// which is worse here: nothing else in the build refers to these values, so
    /// a flag that parsed and was dropped produces a card with no debugger, no
    /// error, and no second symptom. The operator finds out on the board, at
    /// the moment they wanted a debugger.
    #[test]
    fn the_debug_flags_are_optional_and_their_paths_reach_their_fields() {
        let without = argv(&[]);
        assert!(
            matches!(parse_args(&without), Ok(a) if a.gdbserver.is_none() && a.strace.is_none()),
            "a build with neither flag must succeed: it is every card built to date"
        );

        for (flag, value) in [("--gdbserver", "/b/gdbserver"), ("--strace", "/b/strace")] {
            let mut argv: Vec<String> = argv(&[]);
            argv.push(flag.to_string());
            argv.push(value.to_string());
            let parsed = parse_args(&argv);
            let got = match &parsed {
                Ok(a) if flag == "--gdbserver" => a.gdbserver.clone(),
                Ok(a) => a.strace.clone(),
                Err(e) => Some(PathBuf::from(format!("REFUSED: {e}"))),
            };
            assert_eq!(
                got,
                Some(PathBuf::from(value)),
                "{flag} parsed and the path did not reach the field"
            );
            assert!(
                matches!(&parsed, Ok(a) if a.image == "a.img"),
                "{flag}'s value was counted as a positional"
            );

            // And with no value at all: refused, never treated as absent. The
            // silent-absence reading is what would make `--gdbserver` followed
            // by nothing produce a debugger-free card and exit 0.
            let mut dangling: Vec<String> = self::argv(&[]);
            dangling.push(flag.to_string());
            assert!(
                parse_args(&dangling).is_err(),
                "{flag} with no value must be refused, not treated as absent"
            );
        }
    }

    /// `disk` forces root ownership and cannot express anything else without
    /// `fakeroot`. If `system` ever carries a non-root uid, this must be a
    /// decision rather than a silently dropped field.
    #[test]
    fn a_non_root_entry_is_refused_rather_than_quietly_made_root() {
        let node = system::Node {
            path: std::borrow::Cow::Borrowed("/system/bin/x"),
            content: system::Content::Directory,
            mode: 0o755,
            uid: 1000,
            gid: 0,
            reason: "a test",
        };
        assert!(system_entries(&[node]).is_err());
    }

    /// The composition root passes `system::LOGIN_SHELLS` and not something it
    /// made up.
    ///
    /// **Why this reads the source rather than calling `run`.** `run` reads the
    /// firmware, the kernel and two cross-compiled binaries off disk and then
    /// shells out to `sgdisk` and three `mkfs` tools; there is no seam here to
    /// drive it over, and building a card in a unit test would be a different
    /// kind of test entirely. `cmd/rivet-initd`'s
    /// `the_phases_appear_in_run_in_the_order_the_reasoning_requires` takes the
    /// same escape for the same reason and records the same caveat: this proves
    /// the source says the right thing, which is strictly weaker than proving
    /// the program does it, and it is the only thing available.
    ///
    /// It is not idle. `system::LOGIN_SHELLS` is the one input that decides
    /// whether a card can be logged into, and a literal written here instead
    /// would silently bypass `check_login_shells`.
    /// **The slicing is not tidiness.** A first attempt searched the whole
    /// file, and injecting the defect did **not** fail it — because this test's
    /// own body contains the string it was looking for. Searching only `run`'s
    /// text is what makes it able to fail at all.
    #[test]
    fn the_composition_root_takes_the_login_shell_from_the_constant() {
        let source = include_str!("main.rs");
        let start = source.find("fn run() -> Result<String, String> {");
        let end = source.find("/// Render the report for a human.");
        assert!(
            start.is_some() && end.is_some(),
            "cannot locate run() in this file"
        );
        let body = match (start, end) {
            (Some(s), Some(e)) if s < e => source.get(s..e).unwrap_or(""),
            _ => "",
        };
        assert!(!body.is_empty(), "run()'s body did not slice out");
        assert!(
            body.contains("login_shells: system::LOGIN_SHELLS"),
            "run() no longer passes system::LOGIN_SHELLS; a literal here would bypass \
             check_login_shells and ship a card nothing can log in to"
        );
        // The failure this guards against: the
        // supervisor's `rivet-timed` definition names a path, so an unwired
        // field here produces a card whose supervisor spawns something that is
        // not there — the `ENOENT`-on-every-boot case that definition shipped
        // disabled to avoid.
        assert!(
            body.contains("timed: args.timed"),
            "run() parses a TIMED argument and does not pass it to system::tree"
        );
    }

    // ---- one application signing key ---------------------------------------

    /// The labels of every `--trusted-key <label>:<kinds>:<path>` in `script`
    /// whose kinds include `application`, each once.
    fn application_keys(script: &str) -> Vec<&str> {
        let mut labels: Vec<&str> = Vec::new();
        for spec in script.split("--trusted-key ").skip(1) {
            let mut parts = spec.split_whitespace().next().unwrap_or("").splitn(3, ':');
            let (Some(label), Some(kinds)) = (parts.next(), parts.next()) else {
                continue;
            };
            if kinds.split(',').any(|kind| kind == "application") && !labels.contains(&label) {
                labels.push(label);
            }
        }
        labels
    }

    /// **A tripwire: the card build trusts exactly one application signing
    /// key.** `rivet_core::confinement::SUPERVISOR_WINDOWS` grants a window onto
    /// the supervisor by application NAME. That is the same grant as keying it
    /// on the signer only while one key signs every application, because then
    /// an application's name can only have come from that key. A second
    /// application key ends the equivalence: anybody holding it could sign an
    /// application with a windowed name. The day this fails, the window must be
    /// keyed on the signer before the second key ships.
    #[test]
    fn the_card_build_trusts_exactly_one_application_key() {
        const BUILD: &str = include_str!("../../../tools/card/build.sh");
        let keys = application_keys(BUILD);
        assert!(
            !keys.is_empty(),
            "tools/card/build.sh trusts no application key, so this tripwire is no longer reading \
             what the build trusts. Point it at wherever the card's trust store is composed now"
        );
        assert_eq!(
            keys.len(),
            1,
            "the card build trusts {} application signing keys ({keys:?}). The supervisor window \
             (SUPERVISOR_WINDOWS) is granted by application name, which is only as strong as \
             keying it on the signer while one key signs every application. Audit decision D21's \
             signer-keyed window is now owed: key the window on the signer before this ships",
            keys.len()
        );
    }

    /// The tripwire's reader fires on two application keys and ignores a key
    /// that may sign only system kinds — proof it can fail.
    #[test]
    fn the_application_key_reader_counts_only_application_entitlements() {
        let one = "X=\"--trusted-key rivet-system:update,card:$S\"\nY=\"$X --trusted-key rivet-app:application,library:$A\"";
        assert_eq!(application_keys(one), vec!["rivet-app"]);
        let two = format!("{one}\nZ=\"--trusted-key second:apk,application:$B\"");
        assert_eq!(application_keys(&two), vec!["rivet-app", "second"]);
    }
}
