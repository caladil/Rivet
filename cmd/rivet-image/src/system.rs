//! The root filesystem tree of one `/system` slot, as data.
//!
//! `ADR-008` §3 is the authority for the shape; `ADR-018` §3 for the fact that
//! it is `erofs` and therefore **read-only by format**. Nothing here performs
//! I/O: [`tree`] returns a description and [`disk`](crate::disk) materialises
//! it, which is what makes the layout testable without root and without a card.
//!
//! **The `dead_code` allow below is temporary and must come out**, on exactly
//! the terms [`layout`](crate::layout) states: it is here only because `disk`
//! does not consume this yet.
//!
//! ## What this tree actually is
//!
//! Not the contents of `/system`. **The whole root filesystem**, with `/system`
//! as a directory inside it.
//!
//! That follows from three things in the record which only make sense together.
//! `ADR-018` §4 makes `p3`/`p4` the `erofs` slots and `ADR-002` §4 has the
//! kernel mount one of them per `root=`; there is no initramfs and
//! `PLAN.md` §2.1 does not plan one for Stage 0; and `ADR-002` §4 corrects
//! `cmdline.txt` to say that binaries live under `/system/bin`. A kernel that
//! mounts the slot at `/` and then executes `/system/bin/rivet-initd` is only
//! possible if the slot's own root *contains* `system/bin/rivet-initd`. So it
//! does.
//!
//! `cmd/rivet-initd/src/supervise.rs` records this as "a conflict in the
//! record, reported rather than resolved… image assembly is what settles it,
//! and it does not exist yet". This module is image assembly, and [`INIT_PATH`]
//! settles it in favour of `ADR-008`'s later, ruled layout.
//!
//! ## What Stage 0 needs, and nothing else
//!
//! The required binaries and the mount points that cannot be created at
//! runtime. `/etc` ships EMPTY — the runtime projection owns all of it
//! (`ADR-007` §3.5, ruled 2026-08-23). Everything absent from [`tree`] is
//! absent on purpose; the reasons are on the constants and in §"What is
//! deliberately not here".
//!
//! **The count is not written down here**, deliberately: it has moved in both
//! directions and a number in prose goes stale while every test still passes
//! (`AGENTS.md` §7). `the_stage0_image_is_exactly_these_paths` lists the paths,
//! which is the form that cannot drift.
//!
//! **The two that arrived on 2026-08-23** are [`CLI_PATH`] and [`NETD_PATH`],
//! and they are here for different reasons. `rivet` is one of what [`LOGIN_SHELLS`]
//! is allowed to name, so its absence was the reason no card could be logged
//! into. `rivet-netd` was put here to close **the half of a gap this crate
//! can close** — the binary being absent from the card. Starting it was
//! `cmd/rivet-sysd`'s half, and **that half is closed too**:
//! `definitions::system_services` is
//! `vec![netd(), timed(), mdnsd(), mcpd(), bluetoothd()]`, so a card carries
//! `rivet-netd` and the supervisor starts it.
//!
//! **The next arrived later still on 2026-08-23** — [`TIMED_PATH`], `ADR-021`
//! §7's SNTP client. Its service definition predated the binary being
//! installable at all: `cmd/rivet-sysd` shipped it disabled with the missing
//! binary named as the only reason. That constant carries the whole account.
//!
//! **The next arrived 2026-08-24** — [`DEPLOY_PATH`], `ADR-029` §14's
//! installer, and it is the first entry here whose path this crate does not
//! spell: it is re-exported from `rivet_core::deploy`, which both crates read.
//! It is supervised and it runs — the board reports `rivet-deploy` enabled and
//! `done`.
//!
//! **Corrected 2026-09-06, and the shape of the error is worth keeping.** Two
//! of the three paragraphs above said *"the binary is on the card and nothing
//! on the card starts it"*, and one of them offered itself as the sentence the
//! others were measured against. That was true when written and had been false
//! for a fortnight — `netd` joined the supervised set on 2026-08-23 and
//! `rivet-deploy` runs on the hardware. **Nothing failed**, because a doc
//! comment cannot; it was found by reading `definitions.rs` to check something
//! else. `AGENTS.md` §7 is about exactly this, and a paragraph that other
//! paragraphs cite for their own correctness is the expensive kind to get
//! wrong.
//!
//! ## Mount points, and the `EROFS` trap
//!
//! `cmd/rivet-initd/src/boot.rs` calls `mkdir_p` on every target in
//! [`stage0_plan`] before mounting it, and it already knows this can fail:
//! its `mkdir_detail` has a dedicated `EROFS` arm reading "read-only by
//! format". A mount point that lands on this filesystem and is not in the
//! image therefore does not fail obscurely — it produces a `Degraded` boot with
//! a correct message about a card that was built wrong.
//!
//! Which targets those are is not a list to maintain by hand. It falls out of
//! *where the `mkdir` lands at runtime*, which [`provision`] computes:
//!
//! | Target | Lands in | Verdict |
//! | :-- | :-- | :-- |
//! | `/proc` | this filesystem | **must pre-exist** |
//! | `/sys` | this filesystem | **must pre-exist** |
//! | `/sys/fs/cgroup` | sysfs, already mounted | runtime — and the kernel already provides it |
//! | `/dev/pts` | devtmpfs, mounted by the kernel (`ADR-024` §3) | runtime |
//! | `/dev/shm` | devtmpfs | runtime |
//! | `/run` | this filesystem | **must pre-exist** |
//!
//! The three runtime ones are not merely unnecessary in the image — putting
//! them there would be **actively misleading**, because the filesystem mounted
//! over their parent hides them. A directory that is visible in the image and
//! invisible on the running machine is the "silent" failure mode `ADR-007`
//! §3.6 rejects, so [`tree`] omits them and
//! `a_stage0_target_under_a_runtime_mount_is_left_out_because_it_would_be_hidden`
//! keeps them out.
//!
//! ## What is deliberately not here
//!
//! * **Three of the nine paths musl compiles in.** `/etc/resolv.conf`,
//!   `/etc/localtime` and `/etc/services` are absent on purpose and
//!   `rivet_core::etc` argues each one. See [`ETC`].
//!
//!   **Two of those three now exist on a RUNNING machine and still not here**,
//!   which is a distinction worth keeping straight: `/etc/resolv.conf` (ruled
//!   2026-09-01) and `/etc/localtime` (`ADR-021` §9, 2026-09-02) are projected
//!   onto the `/etc` tmpfs at boot. A copy shipped *in the image* at either
//!   path would be masked by that tmpfs and achieve nothing — the same trap
//!   `rivet_core::etc::CA_BUNDLE` records — so their absence from this tree is
//!   unchanged and is now load-bearing for a second reason.
//! * **No `/tmp`.** Nothing in Stage 0 mounts or uses one; `ADR-008` §3's
//!   layout gives runtime state `/run`, which `stage0_plan` does mount.
//! * **No `/lib/firmware`.** `ADR-008` §5.1 confirms the kernel's firmware
//!   loader searches it, but the firmware files themselves are `ADR-004` §8 and
//!   `PLAN.md` §2.3 — Stage 2. An empty `/lib/firmware` would satisfy nothing
//!   and assert that the wireless work had been started.
//! * **No manifest.** `ADR-008` §4.2 item 1 makes the build emit one and calls
//!   it the highest-value organising commitment. It needs file hashes, which
//!   needs I/O, which is `disk`'s side of this seam and not written yet.
//!
//! ## Determinism
//!
//! `ADR-015` §4 signs these artifacts, so the same inputs must produce the same
//! image or the signature covers something nobody can reproduce. This module
//! contributes what it can: every path is a `'static` string, the order is a
//! total sort of those paths, and there is not a timestamp, a hash map or a
//! random value anywhere in it.
//!
//! **That is necessary and not sufficient**, and the rest is `disk`'s.
//! Measured on the build machine 2026-08-22, `mkfs.erofs` (erofs-utils) 1.9.3,
//! over a staging tree holding exactly the twelve entries [`tree`] describes:
//!
//! ```text
//! $ mkfs.erofs a.img stage; mkfs.erofs b.img stage
//! 2775acd2652a1496a44e7469b85bd672c42058e5092d8798aabfd8cdb1355cc2  a.img
//! b8984c3c1d41df51f6595236e12c0e02f8438e5932f97360a13ce718dcdd0531  b.img
//!
//! $ mkfs.erofs --all-root -T0 --all-time -U <fixed> c.img stage   # twice
//! 3362bd66aae13cfe46a20227a4df8762ff6c451248eec88fd6336ea78bd8df52  c.img
//! 3362bd66aae13cfe46a20227a4df8762ff6c451248eec88fd6336ea78bd8df52  d.img
//! ```
//!
//! The naive invocation differs run to run. Three arguments fix it, and all
//! three are needed: [`MKFS_EROFS_OWNERSHIP_ARGS`] for ownership, `-T<n>
//! --all-time` for timestamps, and `-U <uuid>` for the filesystem UUID, which
//! is otherwise randomly generated.

#![allow(dead_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use rivet_core::mount::stage0_plan;
use rivet_core::package::trust_store::TrustStore;

// ---------------------------------------------------------------------------
// The paths that are contracts
// ---------------------------------------------------------------------------

/// The root of the image.
pub const ROOT: &str = "/";

/// PID 1, and the value `cmdline.txt`'s `init=` must name.
///
/// `ADR-002` §4's `init=/sbin/rivet-initd` is stale and says so; `ADR-008` §3
/// and `ADR-016` §3 put the base image's binaries in `/system/bin`. This is the
/// later ruling, and [`boot`](crate::boot) writes the line that has to agree
/// with it.
pub const INIT_PATH: &str = "/system/bin/rivet-initd";

/// The supervisor PID 1 spawns as its single child (`ADR-017` §3.1).
///
/// **This is a second copy of a fact**, which `ADR-011` forbids: the first is
/// `SUPERVISOR_PATH` in `cmd/rivet-initd/src/supervise.rs`. It is duplicated
/// rather than shared because `rivet-initd` is a binary crate with no library
/// target, so nothing can import from it. The fact belongs in `rivet-core`
/// alongside [`stage0_plan`]; moving it is a change to another crate and is
/// reported rather than taken.
/// `the_supervisor_path_matches_the_one_pid_1_execs` is the tripwire until then.
pub const SUPERVISOR_PATH: &str = "/system/bin/rivet-sysd";

/// The CLI, and the binary `/etc/shells` names as this image's login shell.
///
/// **A third copy of the same class of fact** as [`INIT_PATH`] and
/// [`SUPERVISOR_PATH`] — the fourth is `INSTALL_PATH` in
/// `cmd/rivet/src/invocation.rs`, which is what the binary itself answers to
/// in a `-c` string. Same reason as those two: `cmd/rivet` is a binary crate
/// with no library target, so nothing can import from it.
/// `the_cli_path_matches_the_one_the_cli_answers_to` is the tripwire, holding
/// the line on a standing debt tracked since 2026-08-23.
///
/// **One of the three copies went away on 2026-08-24**, which is the fix that
/// item wants rather than another tripwire. The dual-login-shell ruling turned
/// `rivet_core::etc`'s single login-shell constant into a list, and renaming
/// the survivor to `CLI_PATH` made it obvious that this crate was spelling the
/// same literal a second time. It is taken from there now, exactly as
/// [`GIT_PATH`] is taken from `rivet_core::git`.
///
/// **The remaining duplicate is `cmd/rivet`'s `invocation::INSTALL_PATH`**,
/// and it cannot be removed the same way: `cmd/rivet` is a binary crate with
/// no library target, so nothing can import from it and it can import from
/// `rivet-core` — which is the direction the fix has to go, and is a change
/// to that crate rather than to this one.
pub const CLI_PATH: &str = rivet_core::etc::CLI_PATH;

/// nushell, `ADR-005` §5.1's ruled interactive login shell — **installed only
/// when the build is given one.**
///
/// Taken from `rivet_core::etc` for the same reason as [`CLI_PATH`]: the
/// runtime projection probes for this exact path, and a card that installed the
/// binary one character away from where the projection looks would boot, render
/// an `/etc/shells` naming nothing, and refuse every login before
/// authentication.
///
/// **Optional, and that is the ruling that makes `nu` optional rather than a convenience.**
/// The ruling is cheap precisely because a card without `nu` still works: the
/// projection finds [`CLI_PATH`] instead and `permits_a_login` stays true. A
/// required argument here would make that unbuildable and would quietly convert
/// a 42 MiB shell into a hard dependency of the image (`ADR-005` §5.6.2 puts
/// the distributed artifact at roughly +42 % for it).
pub const NUSHELL_PATH: &str = rivet_core::etc::NUSHELL_PATH;

/// Where `cmd/femto` goes — **optional**, on [`NUSHELL_PATH`]'s terms.
///
/// `femto` is a text editor for a **human at a prompt** (`ADR-005` §5.5.1). A
/// card built without it boots, takes its address, accepts a login and installs
/// software — it is a *supported* card, exactly as one without `nu` is, which is
/// the test [`Inputs::deploy`] states for what may be positional and required.
/// *"A card that cannot install an update is not a variant"*; a card with no
/// editor is only a card with no editor.
///
/// **It is 417 KiB stripped**, so unlike `nu` this is not a size argument. It is
/// the same argument as the ruling that makes `nu` optional: optionality is about whether

/// **The boot splash** — `cmd/rivet-splash`, drawn once on `/dev/fb0`.
///
/// **Optional, on [`FEMTO_PATH`]'s terms**: a card with no splash boots
/// identically and shows the console instead, which is what every card before
/// 2026-08-30 did. It is not a positional because its absence is a plainer
/// screen and not a broken machine.
pub const SPLASH_PATH: &str = "/system/bin/rivet-splash";

/// **The BLE observer** — `cmd/rivet-bluetoothd`, `ADR-041`.
///
/// **Optional, and the reason is not [`SPLASH_PATH`]'s.** A splash is optional
/// because a card without one is a plainer screen. This is optional because
/// **the binary is useless without the matching kernel**: it needs `CONFIG_BT`
/// and `CONFIG_BT_HCIUART_BCM`, which arrived in
/// `tools/buildroot/linux/rivet.config` on 2026-09-05 and are in no kernel
/// built before that. Installing it unconditionally would put a program on
/// every card that exits 3 the moment anything enables it.
///
/// The two travel together: `tools/card/build.sh` passes this, and the kernel
/// it passes beside it is one built from the current fragment. A card assembled
/// from an older kernel and this binary is the combination to avoid, and the
/// daemon says which half is missing rather than failing vaguely — see its
/// `exit::NO_SUBSYSTEM`.
///
/// **The service ships DISABLED** ([`crate::system`] does not decide that;
/// `cmd/rivet-sysd`'s definition does), so a card carrying it does not hold the
/// radio until the document says so.
pub const BLUETOOTHD_PATH: &str = "/system/bin/rivet-bluetoothd";

/// **The kiosk menu** — `cmd/rivet-launcher`, `ADR-050` §2.
///
/// **Optional, on [`BLUETOOTHD_PATH`]'s terms rather than [`SPLASH_PATH`]'s**,
/// and the service ships DISABLED for the same reason: whether a machine offers
/// a menu is the **document's** decision, which is what `ADR-050` §2 means by
/// tier 2. A headless machine leaves the launcher off and a kiosk machine adds
/// it — the maintainer's own rule (.rulings/ R-304), and it is `enabled`, a
/// mechanism this project already has, which is why §2.1 says nothing new had
/// to be built for it.
///
/// # It was an application, and it is on the card because a role decides it
///
/// `cmd/rivet-launcher`'s own header records the 2026-09-01 ruling that the
/// launcher makes more sense as an application (.rulings/ R-305), and
/// `deploy/launcher/Cargo.toml` is the manifest that shipped it as one while it
/// was iterated on. `ADR-050` §2
/// supersedes that placement: the launcher is **versioned with the operating
/// system**, and whether a machine offers a menu is a property of the machine
/// rather than of a program somebody installed.
///
/// What that ruling costs is stated in `cmd/rivet-sysd`'s `launcher`
/// definition, which is where the manifest's declarations had to be restated by
/// hand. What it buys is that a kiosk card comes up with a menu on the first
/// boot instead of after a deploy.
pub const LAUNCHER_PATH: &str = "/system/bin/rivet-launcher";

/// **The compositor** — `ADR-058` §4, added 2026-09-20.
///
/// **Optional on [`LAUNCHER_PATH`]'s terms exactly**, and it is the same tier:
/// the image carries it, the supervisor's definition ships it disabled, and the
/// document decides. A headless card carrying it behaves as a card without it.
///
/// **It is the first binary in `/system/bin` that is DYNAMICALLY LINKED.**
/// `ADR-003` §5 rules static musl and `tools/card/build.sh`'s `must_be_static`
/// holds every other one of Rivet's own to it; this is the exception
/// `ADR-058` §5 records, forced by Mesa staging no archive for `libgbm`. The
/// card's own gate is `must_be_runnable`, which asks the question that
/// actually matters — *can this exec on the card being written* — and refuses
/// if `libxkbcommon.so.0`, `libgbm.so.1` or `libgcc_s.so.1` is not on it.
pub const DISPLAYD_PATH: &str = "/system/bin/rivet-displayd";

/// **The path proxy, the machine's front door** — `ADR-060`.
///
/// Optional on [`LAUNCHER_PATH`]'s terms: the supervisor ships it disabled and
/// the document's role turns it on, so a card without it is a card whose
/// machine serves nothing on 443. `tools/card/build.sh` always passes it, and
/// its listing gate refuses a card that lacks it.
///
/// **Read out of `rivet-core`** rather than spelled here, unlike its
/// neighbours: `cmd/rivet-sysd` executes the same constant, so the two cannot
/// disagree and no tripwire is needed to hold them together.
pub const PROXYD_PATH: &str = rivet_core::service::PROXYD_PATH;

/// **The name service** — `ADR-047`. Optional on [`PROXYD_PATH`]'s terms: the
/// supervisor ships it disabled and a document turns it on. Read out of
/// `rivet-core` for the same reason.
pub const DNSD_PATH: &str = rivet_core::service::DNSD_PATH;

/// **The WPA supplicant** - `ADR-004` §3's permanent Tier C dependency.
///
/// **Optional, on [`SPLASH_PATH`]'s terms.** A card without it is a card with
/// no wireless, which is a supported machine: `ADR-004` §2 brings up wired
/// first and §2.4 keeps the wired interface afterwards.
///
/// Buildroot leaves it at `target/usr/sbin/wpa_supplicant`. It lands in
/// `/system/bin` like every other binary, Rust and C alike - `ADR-008` §3
/// rejects the FHS, so `usr/sbin` is not a place on a Rivet machine.
pub const SUPPLICANT_PATH: &str = "/system/bin/wpa_supplicant";

/// `wpa_cli`, the supplicant's own control-socket client.
///
/// **A BRING-UP tool, and the reason is method rather than convenience.**
/// `ADR-004` §4 has `cmd/rivet-netd` driving the control socket itself, so
/// nothing on a finished machine needs this. What it buys is that the C side
/// can be proven to associate INDEPENDENTLY of Rivet's client - so a failure
/// has one candidate cause instead of two, which matters when the feedback
/// loop is an image cycle. A fair thing to drop once wireless works.
pub const WPA_CLI_PATH: &str = "/system/bin/wpa_cli";

/// **The DHCP client** - BusyBox `udhcpc`, `ADR-004` §8.
///
/// Named `udhcpc` rather than `busybox` because that is what it IS: the binary
/// is built with exactly one applet (`tools/buildroot/busybox/busybox.config`),
/// and with one applet compiled in the multiplexer dispatches to it whatever
/// `argv[0]` says. A file called `busybox` would advertise three hundred
/// programs that are not in it.
///
/// **Optional**, on the supplicant's terms: a machine with a static address
/// needs no DHCP client, and that is what every Rivet board has run on so far.
pub const UDHCPC_PATH: &str = "/system/bin/udhcpc";

/// **The program `udhcpc` execs when a lease changes** — `cmd/rivet-dhcp-hook`.
///
/// # This string is compiled into BusyBox, and the two must match
///
/// `tools/buildroot/busybox/busybox.config` sets
/// `CONFIG_UDHCPC_DEFAULT_SCRIPT` to this exact path, because Buildroot's own
/// default is `/usr/share/udhcpc/default.script` — an FHS path `ADR-008` §3
/// means Rivet does not have. So the agreement is between a C build-time
/// constant and this Rust constant, which is the weakest kind of agreement
/// there is: nothing checks it and both look right on their own.
///
/// What guards it is [`SystemError::DhcpClientWithoutItsHook`]: a card carrying
/// `udhcpc` and not this binary is refused at build time. That does not prove
/// the two STRINGS match — only a board can — but it removes the failure that
/// would otherwise be silent, where DHCP runs, gets an answer, and execs
/// nothing.
pub const DHCP_HOOK_PATH: &str = "/system/bin/rivet-dhcp-hook";

/// **Where a dynamically-linked program's libraries live.**
///
/// Not an FHS concession either, for [`LOADER_PATH`]'s reason one step on:
/// musl's loader searches `/lib`, `/usr/local/lib` and `/usr/lib` when a binary
/// carries no `DT_RUNPATH` and the environment names no path. This is the first
/// of those, it already exists because the loader is in it, and a library here
/// needs no cooperation from anything.
pub const LIB_DIR: &str = "/lib";

/// **The dynamic loader.** Shared Rivets, 2026-08-31.
///
/// The second genuine kernel-side path in this image, and it belongs beside
/// [`FIRMWARE_BRCM_DIR`] for exactly that reason: like `/lib/firmware`, it is
/// not an FHS concession this project chose to make. It is the string the cross
/// toolchain writes into a dynamically-linked binary's `PT_INTERP`, and the
/// **kernel** opens it, before the program's first instruction. A file anywhere
/// else is a file no program can be linked against.
///
/// `ADR-003` §5's static ruling is untouched: **every Rivet binary is still
/// statically linked** and none of them uses this. It is here for applications,
/// which `docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §4
/// makes the whole point of shared libraries — and it is the entirety of what
/// "ship the loader" (that document's Phase B) turned out to mean on musl,
/// because musl's loader **is** its libc.
///
/// # It is also the platform's public ABI surface
///
/// `rivet_core::package::library::PUBLIC_LIBRARIES` is `["libc.so"]`, and this
/// file is that library. Android's `public.libraries.txt` lesson (§4.2): every
/// symbol the platform promises is one it must keep promising across image
/// updates, so the list is one entry and everything else an application links
/// arrives in its own closure.
pub const LOADER_PATH: &str = "/lib/ld-musl-aarch64.so.1";

/// Where the kernel's firmware loader looks - **the one genuine kernel-side
/// path**, `ADR-008` §5.1.
///
/// Not an FHS concession: the default is compiled into the kernel, and while
/// `firmware_class.path` can override it, *"`/lib/firmware` is the path of
/// least resistance rather than a free choice"*.
pub const FIRMWARE_BRCM_DIR: &str = "/lib/firmware/brcm";

/// The CYW43455's firmware, under the exact names the driver asks for.
///
/// # These names were measured, not guessed
///
/// `ADR-004` §8.1 read them off a Raspberry Pi 4 Model B Rev 1.1 running stock
/// Raspberry Pi OS on 2026-08-22: the driver takes the base name
/// `brcm/brcmfmac43455-sdio` and appends **the first device-tree compatible
/// string**, which on this board is `raspberrypi,4-model-b`.
///
/// # Three files, and a missing one fails QUIETLY
///
/// `ADR-004` §8's warning is that a missing `clm_blob` does not fail - it makes
/// association *misbehave*. There is also a fourth the driver asks for and does
/// not need (`txcap_blob`), and §8.1 measured that its absence is harmless, so
/// it is deliberately not here.
///
/// # The symlink problem, solved by arithmetic rather than machinery
///
/// §8.1 warned that on Raspberry Pi OS every one of these is a **symlink** into
/// `cypress/`, and that Rivet *"must ship real files under the names the driver
/// requests"*. Buildroot's `brcmfmac_sdio-firmware-rpi` package installs them
/// the same way - its `.mk` copies with `--no-dereference`. **Nothing extra is
/// needed**: [`Content::CopiedFrom`] is materialised with `std::fs::read`,
/// which follows the link and yields the target's bytes, so what lands on the
/// card is a real file. A link shipped as a link would dangle, and this is why
/// it cannot.
pub const FIRMWARE_BIN: &str = "/lib/firmware/brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.bin";
/// The country/regulatory blob. See [`FIRMWARE_BIN`] - absence is quiet.
pub const FIRMWARE_CLM: &str =
    "/lib/firmware/brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.clm_blob";
/// The board's NVRAM parameters. See [`FIRMWARE_BIN`].
pub const FIRMWARE_NVRAM: &str =
    "/lib/firmware/brcm/brcmfmac43455-sdio.raspberrypi,4-model-b.txt";

/// **The Bluetooth patchram, added 2026-09-05 with `R-144` tier 1.**
///
/// The other half of the same CYW43455 the three files above are firmware for,
/// and it very nearly did not ship: `R-144`'s work enabled Buildroot's
/// `BR2_PACKAGE_BRCMFMAC_SDIO_FIRMWARE_RPI_BT` and taught the kernel
/// `CONFIG_BT_HCIUART_BCM`, but nothing added the file to [`FIRMWARE_FILES`] —
/// **a hard-coded list of exactly three.** A card built in that state carries a
/// Bluetooth driver and no firmware, `hci_bcm` fails at `hdev->setup`, and the
/// controller never appears. That looks precisely like a dead radio, which is
/// the wrong thing to spend a morning on.
///
/// # The name was MEASURED, on 2026-09-05, and it is this one
///
/// `hci_bcm` composes the same way `brcmfmac` does — a base name plus the
/// device tree's first compatible string. That was documentation until a board
/// booted this kernel and said so itself:
///
/// ```text
/// Bluetooth: hci0: BCM4345C0 (003.001.025) build 0000
/// Bluetooth: hci0: BCM4345C0 'brcm/BCM4345C0.raspberrypi,4-model-b.hcd' Patch
/// Bluetooth: hci0: BCM43455 37.4MHz Raspberry Pi 3+-0190
/// Bluetooth: hci0: BCM4345C0 (003.001.025) build 0382
/// ```
///
/// **The build number moving from 0000 to 0382 is the patch taking effect** —
/// the radio is running patched firmware and not the stock image, which is a
/// stronger statement than the file having been opened.
///
/// ~~The bare `BCM4345C0.hcd` shipped beside it as a fallback, because neither
/// name had been read off a board and 62 KiB was a cheap way to let the first
/// boot answer the question instead of failing it.~~ **Removed the same day,
/// which is what that sentence promised.** The driver asked for the
/// compatible-suffixed name and never reached its fallback, so the second copy
/// is 62 KiB of `ADR-009` §4's *carrying what nothing uses* — and the record of
/// why it was briefly there is this paragraph rather than a file on a card.
pub const FIRMWARE_BT_COMPATIBLE: &str =
    "/lib/firmware/brcm/BCM4345C0.raspberrypi,4-model-b.hcd";

/// The four names as they appear inside the directory a build is given, paired
/// with where each lands.
///
/// **One list rather than a pair repeated at each site**, `ADR-011`: the source
/// basename and the destination share a string, and writing it twice is how
/// they come to disagree.
pub const FIRMWARE_FILES: &[(&str, &str)] = &[
    ("brcmfmac43455-sdio.raspberrypi,4-model-b.bin", FIRMWARE_BIN),
    (
        "brcmfmac43455-sdio.raspberrypi,4-model-b.clm_blob",
        FIRMWARE_CLM,
    ),
    (
        "brcmfmac43455-sdio.raspberrypi,4-model-b.txt",
        FIRMWARE_NVRAM,
    ),
    // **The Bluetooth half** — one name, the one a board asked for. See
    // `FIRMWARE_BT_COMPATIBLE` for the journal lines that measured it.
    (
        "BCM4345C0.raspberrypi,4-model-b.hcd",
        FIRMWARE_BT_COMPATIBLE,
    ),
];

/// a card without it is a machine somebody would knowingly build.
pub const FEMTO_PATH: &str = "/system/bin/femto";

/// The directory `rivet_core::etc::CA_BUNDLE_SOURCE` lives in.
///
/// Spelled here rather than derived from the path, because `Node::dir` wants a
/// `&'static str` and a directory that disagreed with the file inside it would
/// produce an image with an empty directory and a file nothing can reach.
pub const CA_BUNDLE_DIR: &str = "/system/ssl";

/// The directory `rivet_core::etc::SYSTEM_FONT` lives in.
///
/// **Aliased rather than spelled again, 2026-09-05.** It was a second literal
/// until `rivet_core::confinement` grew a rule for the same directory
/// (`ADR-042` §4.2) and made it a third; `ADR-011` says what three spellings of
/// one path are. `rivet_core::etc::SYSTEM_FONT_DIR` is the authority and this
/// is a name for it, so the image and the sandbox cannot disagree about where
/// a Rivet's type is.
pub const FONT_DIR: &str = rivet_core::etc::SYSTEM_FONT_DIR;

/// The MCP control plane's listener — `ADR-035` phase 1, built 2026-08-28.
///
/// **Positional and required since 2026-08-28**, and it spent one day as an
/// option. The doc that stood here argued the other way, and named the exact
/// condition that has now fired: *"it becomes a positional the day the
/// supervisor names it"*. `cmd/rivet-sysd`'s `mcpd` definition names it.
///
/// **Required, even though that service ships DISABLED**, and the two facts
/// are not in tension — keeping them apart is the point. `rivet-mcpd`'s
/// `enabled: false` is about the **roadmap**: phase 1 binds loopback, so a
/// machine without it has lost no way in, and phase 2 flips it. That is a
/// different claim from `rivet-timed`'s old `false`, which was about the
/// **image** — its binary was not on the card.
///
/// If this stayed optional, a card could be built where `enabled: false` meant
/// *both* at once, and no one reading a machine could tell which. Requiring the
/// binary leaves exactly one reason on the definition, which is `ADR-011`
/// applied to a service's own state rather than to a path.
///
/// `PLAN.md` §72.3 is why this constant exists at all: adding the binary to
/// `tools/card/build.sh`'s `$OURS` built and stripped it and **did not install
/// it**, because every binary reaches the image through this module and that
/// one had no path to reach.
pub const MCPD_PATH: &str = "/system/bin/rivet-mcpd";

/// The network daemon (`ADR-014` §2), which at Stage 1 converges one interface
/// and exits rather than staying up (`PLAN.md` §2.2).
///
/// **Installed 2026-08-23**, closing half of a standing gap — *"`cmd/
/// rivet-netd` is in the workspace and in nothing else."* The other half,
/// `rivet-sysd` starting it, closed the same day: `cmd/rivet-sysd::definitions::netd`
/// is the service definition, so a card built today carries this binary
/// and `rivet-sysd` runs it.
pub const NETD_PATH: &str = "/system/bin/rivet-netd";

/// The mDNS responder — `PLAN.md` §23, built 2026-08-26.
///
/// **Required rather than optional**, on [`TIMED_PATH`]'s terms exactly:
/// `cmd/rivet-sysd` defines the service and names this path, so a card without
/// the binary would be one whose supervisor names a path nothing put there. It
/// ships **disabled** and the document turns it on, which is a different fact
/// from whether it is present — and keeping the two separate is the whole
/// reason this is not an optional argument.
pub const MDNSD_PATH: &str = rivet_core::service::MDNSD_PATH;

/// The git binary — `ADR-009` §6's Tier B, `PLAN.md` §43's survey.
///
/// **One file, not the 171 Buildroot installs.** `usr/libexec/git-core` holds
/// 171 entries of which 147 are hardlinks to this same 3.87 MB inode; a naive
/// copy costs 554 MB. Everything Rivet needs is a *builtin* reached as
/// `git <verb>`, measured on 2026-08-26 by tracing a real push, so the image
/// carries the one executable.
///
/// **Required because the control plane's smart-HTTP handler reaches it**:
/// `git-http-backend` runs `git` rather than finding `upload-pack` by
/// `--exec-path`, so an optional argument here would produce a card that
/// answers a clone and serves nothing. It costs 3.87 MB on every card and that
/// is the price of the failure being impossible rather than merely unlikely.
///
/// **Not built by this workspace**, and for `ADR-009` §6's reason: a C
/// mechanism is Buildroot's. `tools/buildroot/build.sh` leaves it at
/// `target/usr/bin/git`, and it arrives here as an input path exactly as the
/// firmware and the kernel do.
///
/// It is under `/system/bin` beside the Rust binaries, and `ADR-008` §3 is what
/// says that is right rather than merely convenient. §3's table separates paths
/// by **ownership and lifetime**, not by language or by vendor: `/system` is
/// "the Rivet image: binaries, image defaults", read-only, "replaced wholesale"
/// by an update, and a statically linked binary built by Buildroot has exactly
/// that lifetime. The alternative that suggests itself — a separate
/// `/system/mechanisms` or an FHS-shaped `/usr/bin` — would encode a
/// distinction the layout deliberately does not make, and `ADR-009` §6 warns
/// against precisely that: "if Buildroot's own root filesystem conventions ever
/// reach the image, the boundary has been crossed — those conventions are the
/// FHS, which `ADR-008` §3 rejects."
pub const GIT_PATH: &str = rivet_core::git::BINARY;

/// `git-http-backend`, git's smart-HTTP server — `ADR-035` §4's phase 3.
///
/// **Required, on [`GIT_PATH`]'s terms**: `ADR-035`'s phase 3 moved git onto
/// this and phase 5 removed the transport the old path used, both done. So
/// this is no longer the future-proofing it was written as — it is **the only
/// way a repository on this card can be reached at all**, and a card without it
/// is a card whose repositories do not exist. It costs 2.33 MB and that is the
/// price of the failure being impossible — the same trade [`GIT_PATH`] made.
///
/// **A second file rather than a second name for the first.** `PLAN.md` §84.1
/// measured the inodes: `git` and `git-upload-pack` are one binary and this is
/// another. Assuming otherwise is what made `git http-backend` answer *"not a
/// git command"* on a running board.
pub const HTTP_BACKEND_PATH: &str = rivet_core::git::HTTP_BACKEND;

/// `ADR-021` §7's SNTP client — **the binary whose gap was recorded before it
/// was closed.**
///
/// `cmd/rivet-sysd/src/definitions.rs` defined this service on 2026-08-23 and
/// shipped it `enabled: false`, with the reason stated as a fact about *this
/// crate* rather than a judgement about clocks: **`cmd/rivet-image` did not
/// install the binary.** An enabled definition would therefore have had the
/// supervisor spawn a path that was not on the card, get `ENOENT`, and —
/// correctly — refuse loudly and permanently, on every boot of every board,
/// about a service the operator never asked for.
///
/// **That stopped being true with this constant**, and the supervisor's
/// tripwire `the_timed_path_matches_the_image_if_the_image_installs_it` was
/// written to notice exactly this: it greps *this file* for the declaration
/// below, prints that the binary is not installed when it cannot find one, and
/// asserts the two paths agree when it can. It can now.
///
/// **Do not quote that grep's marker text anywhere above this line.** It splits
/// on the declaration's own opening and takes what follows, so a doc comment
/// reproducing it verbatim makes the tripwire read prose instead of a path — and
/// it did, once, before this note existed.
///
/// **A second copy of a `/system/bin` path**, the other being the supervisor's
/// own `TIMED_PATH`. Same reason as every other one — a binary crate has no
/// library target — and the same debt, standing since 2026-08-23.
///
/// Unlike [`NETD_PATH`], the sentence *"the binary is on the card and nothing
/// on the card starts it"* is **not** what this one leaves behind: the service
/// definition exists already, so installing the binary is the last of the two
/// halves rather than the first.
pub const TIMED_PATH: &str = "/system/bin/rivet-timed";

/// Where `cmd/rivet-deploy` goes. **The installer** — `ADR-029` §14,
/// the maintainer's 2026-08-24 ruling.
///
/// **An eighth `/system/bin` path in this workspace and the FIRST that is not a
/// duplicate**, which is worth stating because every one before it is.
/// That debt has stood since 2026-08-23 — a
/// binary crate has no library target, so `cmd/rivet-image` and
/// `cmd/rivet-sysd` each spell the other's paths and a test greps the
/// other crate's source to hold them together. This one is
/// `rivet_core::deploy::PROGRAM_PATH`, re-exported: both crates read the same
/// constant, so there is nothing to drift and nothing to grep.
///
/// The reason it could be done here and not for the others is not that this
/// path is special — it is that `rivet_core::deploy` had to exist anyway, for
/// `rivet get install` to know which service's records to read. Doing the
/// same for the six above is the remaining work on that debt.
///
/// **It is `/system` rather than `/state` on purpose.** `ADR-008` §3 puts
/// read-only vendor content there, replaced wholesale by an update, and an
/// installer living on the partition it writes into would be a machine that
/// cannot repair itself after the write it was performing went wrong.
///
/// **Nothing on the card starts it**, and that is the maintainer's 2026-08-24
/// install-request ruling's open half rather than an oversight in this crate: `cmd/rivet-sysd`'s
/// `definitions::deploy` is the service definition and it takes a payload path,
/// which nothing can supply until the channel is ruled. Putting the binary on
/// the card is the half that can be done now — and it is the half that has to
/// be done first, because a channel cannot be tested against a card that does
/// not carry the program.
pub use rivet_core::deploy::PROGRAM_PATH as DEPLOY_PATH;

/// `gdbserver` — **installed only when the build is given one**, and the reason
/// it is optional is not the same as `nushell`'s.
///
/// `ADR-005` §7 is the ruling this implements, and §7.4 is the part that is
/// still the maintainer's: whether a debugger belongs on every card or only on
/// some. **This constant does not answer that.** What it does is make an image
/// with one *buildable*, so the question can be decided against a measured cost
/// rather than an estimate — which is why the argument is optional and its
/// absence produces exactly the card every earlier build produced.
///
/// It is `/system/bin` for the reason [`GIT_PATH`] is, and the reason is
/// the layout's rather than the language's: `ADR-008` §3's table separates paths
/// by **ownership and lifetime**, `/system` is read-only vendor content replaced
/// wholesale by an update, and a statically linked debug stub built by Buildroot
/// has precisely that lifetime. `ADR-009` §6's warning applies unchanged — the
/// FHS-shaped `/usr/bin` that Buildroot itself installs this to is exactly the
/// convention that may not reach the image.
///
/// **The name is not a dispatch decision.** `gdbserver`'s `main` in
/// `gdbserver/server.cc` reads its arguments and never looks at
/// `basename(argv[0])`, so there is no second program hiding behind the name
/// and nothing to refuse — which is not true of every multi-call binary, and is
/// therefore worth having checked rather than assumed.
///
/// **Nothing on the card runs it**, and nothing should: a debug stub started by
/// the supervisor would be a listening socket on every board, which is
/// `ADR-028`'s question and not this crate's. It is spawned by an operator who
/// has already logged in.
pub const GDBSERVER_PATH: &str = "/system/bin/gdbserver";

/// `strace` — **installed only when the build is given one**, on the same terms
/// as [`GDBSERVER_PATH`].
///
/// It earns its place separately from the debugger rather than as a companion
/// to it. Rivet drives C mechanisms it did not write and cannot step through
/// (`ADR-009` §3), and when one of them fails the journal records
/// Rivet's account of the failure — an exit status, a message the mechanism
/// chose to print. Which `syscall` it actually failed on, against which path,
/// with which `errno`, is a fact only the kernel has, and `strace` is how it is
/// read. `ADR-005` §7.2 has the worked example.
pub const STRACE_PATH: &str = "/system/bin/strace";

/// Where `ADR-016` §3 mounts system extensions.
///
/// An **empty directory in the image**, and `ADR-018` §6 explains why that
/// detail is worth catching now: mount points cannot be created on a read-only
/// filesystem, so `/system/ext` is a tmpfs over a directory that has to already
/// be here, and extensions mount inside *that*. Without it the first extension
/// is impossible to mount and the reason is non-obvious.
///
/// Stage 0 has no extensions. The directory costs one inode and its absence
/// costs a reflash.
pub const EXT_MOUNT_POINT: &str = "/system/ext";
/// Where a machine's trust store lives. `ADR-028` §5.5.6.
///
/// **Re-exported from `rivet_core::package::trust_store::PATH`**, not
/// declared here — 2026-08-24. `cmd/rivet-deploy` has to read the same file
/// this writes, and the two live in binaries that cannot see each other, so a
/// second spelling of the path was `ADR-011` §3 waiting to happen. The
/// reasoning that put it there is kept below because it is still the reasoning.
pub use rivet_core::package::trust_store::PATH as TRUST_STORE_PATH;

/// Where the image records what its binaries execute on — `ADR-027` §3's
/// `<arch>`. **Re-exported from `rivet_core::package::arch::PATH`**, for
/// [`TRUST_STORE_PATH`]'s reason: this tool writes it and two other binaries
/// read it.
pub use rivet_core::package::arch::PATH as ARCH_PATH;

/// The compatibility shim, and it is **no longer empty** — 2026-08-23.
///
/// `ADR-008` §5.1 makes `/etc` **mandatory**: musl has nine paths compiled in as
/// string literals (`rivet_core::etc::MUSL_ETC_PATHS`, confirmed a second time
/// against the `libc.a` this project actually links). `ADR-007` §3.5 makes its
/// contents **generated output, never authored**.
///
/// This constant used to carry the argument for shipping an empty directory, and
/// two clauses of that argument were wrong. They are quoted rather than deleted,
/// per `AGENTS.md` §2:
///
/// > "the image's job is the mount point and nothing else … a file written here
/// > is **hidden the moment the tmpfs is mounted over `/etc`**."
///
/// **The first is wrong because "generated" does not mean "generated at
/// runtime".** These files are generated — from
/// `rivet_core::etc::image_etc`, which is a projection of constants exactly as
/// the runtime one will be a projection of the configuration document. Nothing
/// here is authored, nothing here is editable, and `ADR-011` is satisfied
/// because the two projections render *different facts* from *one* definition
/// apiece. `rivet_core::etc`'s module documentation carries the full argument
/// and the ruling.
///
/// **The second is wrong today and conditional tomorrow.** Nothing mounts
/// anything at `/etc`: `stage0_plan` has six entries and
/// `nothing_is_mounted_over_etc_so_these_files_are_the_live_ones` pins that. The
/// day a tmpfs *is* mounted there, the generator behind it renders
/// `image_etc()` as well — so the projection is a superset by construction and
/// the concern dissolves rather than being managed.
pub const ETC: &str = "/etc";

/// The binary this image permits as a login shell, or `None` if it ships none.
///
/// Measured on real hardware: a login is rejected **before
/// any credential is looked at** unless the account's shell is listed in
/// `/etc/shells` — `getusershell()` is what reads it. So this value is what
/// `rivet_core::etc` writes into that file, and it is the difference between a
/// board that can be logged into and one that cannot.
///
/// **`Some(`[`CLI_PATH`]`)`.** It was `None` until 2026-08-23, when `cmd/rivet`
/// became a workspace member and [`Inputs`] gained the path. Naming
/// `/system/bin/rivet` here before the binary existed would have produced an
/// `/etc/shells` pointing at a file that is not in the image: a card that looks
/// provisioned for login and fails at `exec`, which is precisely `AGENTS.md`
/// §6's "wired up" reported as "works". [`check_login_shells`] now runs against
/// a tree that contains the binary, so the *accepting* arm of that guard is the
/// one `main` exercises on every build.
///
/// **The image itself ships no `/etc`** (`ADR-007` §3.5), so this constant
/// decides what the *runtime projection* renders rather than what is on the
/// card: `cmd/rivet-sysd`'s `etc` module is what writes `/etc/shells` onto a
/// tmpfs at boot, and this is one of the facts it needs.
///
/// It is a constant *and* an [`Inputs`] field, which looks redundant and is not:
/// `main` passes this value and nothing else does, while a test can pass any
/// value it likes. A guard reachable only through a constant that was `None` is
/// a guard no test could drive — and injecting its removal proved exactly that,
/// silently, before the field existed (`AGENTS.md` §4). **The field earns its
/// keep in the other direction now**: the constant reaches the accepting arm, so
/// the *refusing* arm is the one that needs an injectable value, and
/// `tree_refuses_to_build_an_image_whose_login_shell_it_does_not_contain` uses
/// it to name ~~`nushell` — the shell `ADR-005` §5.1 actually rules in, and
/// which this image does not contain~~ a shell this image does not contain.
///
/// ---
///
/// # A list since 2026-08-24, and the guard it feeds got *weaker* on purpose
///
/// The dual-login-shell ruling made `nu` `root`'s login shell and `cmd/rivet` a second
/// permitted one, so the runtime projection now probes a **list** of candidates
/// and takes whichever are present. This constant is that list, taken from
/// `rivet_core::etc::LOGIN_SHELL_PATHS` rather than restated, because the
/// probe and the build must be looking at the same paths or the guard below
/// checks something no machine will ever ask for.
///
/// **The guard used to mean "the named shell must be in the image" and now
/// means "at least one candidate must be".** That is a real weakening and it is
/// the correct one: a candidate list is a list of things that *may* be there,
/// and that same ruling requires exactly that a missing `nu` be an ordinary
/// outcome rather than a build failure. What the guard still catches is the
/// failure it was written for — an image nothing can log into — because a tree
/// containing none of the candidates produces an `/etc/shells` that names
/// nothing, whichever entry is missing.
///
/// The old name is gone rather than kept as an alias: `LOGIN_SHELL`, singular,
/// asserted there was one, and `ADR-011`'s objection to two names for one fact
/// applies with more force when one of the two names is wrong.
pub const LOGIN_SHELLS: &[&str] = rivet_core::etc::LOGIN_SHELL_PATHS;

/// `ADR-008` §3's persistent partition — `p5` in [`layout`](crate::layout).
///
/// **Taken from `rivet-core` rather than spelled again**, because since
/// 2026-08-23 `cmd/rivet-initd` mounts it and the two would otherwise be two
/// literals for one fact (`ADR-011`).
///
/// It is in the tree because it is the one thing that cannot be added later
/// without rebuilding the image: `/state/config` (`ADR-008` §3.1), `/state/ext`
/// (`ADR-018` §6) and `/state/home` (`ADR-008` §7.3) all need this mount point
/// to exist, and the filesystem holding it is read-only.
pub const STATE_MOUNT_POINT: &str = rivet_core::state::STATE_MOUNT_POINT;

/// Where the kernel mounts devtmpfs, before PID 1 exists.
///
/// `ADR-024` §3's `DEVTMPFS_MOUNT` means the kernel does this itself and there
/// is no udev (`ADR-005` §2.1). **The kernel still needs the directory**: it
/// mounts onto `/dev` on the root filesystem it has just mounted, and if the
/// path is not there the mount fails and every device node is missing.
///
/// **Amended 2026-08-24: it IS in [`stage0_plan`] now.** The sentence that
/// stood here said it was not, "by the time PID 1 runs it is already done" —
/// true on a board and false under the emulator, where the root is an initramfs
/// and the kernel's automount does not apply. PID 1 therefore mounts it, and
/// the directory is still required here for the same reason as before: the
/// mount needs somewhere to land.
pub const DEV_MOUNT_POINT: &str = "/dev";

// ---------------------------------------------------------------------------
// Ownership and modes
// ---------------------------------------------------------------------------

/// Everything in the image is owned by `root`.
///
/// `ADR-008` §7.3 keeps the full Unix multi-user model and `ADR-015` §5.1 gives
/// applications their own UIDs — but neither of those touches the *base image*,
/// which is vendor content that no application may write and that `ADR-018` §3
/// makes unwritable anyway. `/state` is where per-UID ownership starts to
/// matter, and `/state` is not in this tree.
pub const ROOT_UID: u32 = 0;
/// See [`ROOT_UID`].
pub const ROOT_GID: u32 = 0;

/// Directories: readable and traversable by everyone, writable by nobody.
///
/// The "writable by nobody" half is free — this is `erofs` — but the mode is
/// still stated rather than left to the umask, because a mount point's mode
/// survives into the image and a `0700` `/system` would be a machine on which
/// no service can find its own binary.
pub const DIR_MODE: u32 = 0o755;

/// Executables. Not setuid, and there is nothing in the base image that wants
/// to be: `ADR-017` §3.1 runs the supervisor as root already.
pub const EXEC_MODE: u32 = 0o755;

/// What `mkfs.erofs` needs so that an image built by an unprivileged user is
/// still owned by `root`.
///
/// **This is the whole answer to "what does root-owned mean when the builder is
/// not root".** `mkfs.erofs` copies the staging tree's uid and gid into the
/// image, so a card built in WSL2 by an ordinary account would boot with
/// `/system/bin/rivet-initd` owned by uid 1000 — a uid that does not exist on
/// the machine, on a filesystem where it cannot be repaired.
///
/// The alternatives are worse in the ways one would expect: `fakeroot` adds a
/// dependency and a wrapper process, and building as `root` makes the whole
/// build a privileged operation to fix two integers.
///
/// **Measured, not assumed** (`AGENTS.md` §5), on the build machine
/// 2026-08-22 with erofs-utils 1.9.3, building as uid 1000 and reading the
/// inode back out of the image with `dump.erofs`:
///
/// The transcript names `rivet-init`, which is what PID 1 was called that
/// day; it became `rivet-initd` on 2026-08-24 and the transcript is left as
/// it was typed and answered. [`INIT_PATH`] is the live value.
///
/// ```text
/// $ mkfs.erofs a.img stage
/// $ dump.erofs --path=/system/bin/rivet-init a.img
/// Uid: 1000   Gid: 1000  Access: 0755/rwxr-xr-x
///
/// $ mkfs.erofs --all-root c.img stage
/// $ dump.erofs --path=/system/bin/rivet-init c.img
/// Uid: 0   Gid: 0  Access: 0755/rwxr-xr-x
/// ```
///
/// Note what the second line of each pair also shows: **modes are not touched
/// by `--all-root`**, which is why [`Node::mode`] is carried per entry and
/// applied by [`disk`](crate::disk) to the staging tree before this runs.
pub const MKFS_EROFS_OWNERSHIP_ARGS: &[&str] = &["--all-root"];

// ---------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------

/// What a [`Node`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Directory,
    /// A file copied byte-for-byte from a host path.
    ///
    /// The binaries are **inputs**, not something this crate builds, and the
    /// path to one is an argument rather than a constant (`AGENTS.md` §8.3).
    CopiedFrom(PathBuf),
    /// Bytes this build produced, from a definition rather than from a file.
    ///
    /// The `/etc` files, and nothing else. Held inline rather than written to a
    /// scratch file and copied, so that the whole tree stays a value this module
    /// can hand over without touching a filesystem — which is what keeps
    /// `ADR-014` §3's no-I/O rule true of it and lets every decision here be
    /// tested without a card.
    Generated(Vec<u8>),
}

/// One entry in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Absolute, and where it lands on the booted machine.
    ///
    /// **A `Cow`, and it was `&'static str` until Mesa.** Every node in this
    /// image was a compile-time constant, which is a property worth having and
    /// was worth keeping for as long as it was true. A vendor shared library's
    /// path is its SONAME and that is read off a file, so it cannot be. The
    /// constructors take `impl Into<Cow<'static, str>>`, so a constant is still
    /// borrowed and costs nothing.
    pub path: std::borrow::Cow<'static, str>,
    pub content: Content,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// Why this entry exists, in one sentence.
    ///
    /// Carried as data rather than as a comment so that it survives into
    /// whatever `disk` prints. An image is a thing somebody stares at during a
    /// failed bring-up, and "why is this directory here" is the question they
    /// will have.
    pub reason: &'static str,
}

impl Node {
    fn dir(path: impl Into<std::borrow::Cow<'static, str>>, reason: &'static str) -> Self {
        Self {
            path: path.into(),
            content: Content::Directory,
            mode: DIR_MODE,
            uid: ROOT_UID,
            gid: ROOT_GID,
            reason,
        }
    }

    fn executable(
        path: impl Into<std::borrow::Cow<'static, str>>,
        source: PathBuf,
        reason: &'static str,
    ) -> Self {
        Self {
            path: path.into(),
            content: Content::CopiedFrom(source),
            mode: EXEC_MODE,
            uid: ROOT_UID,
            gid: ROOT_GID,
            reason,
        }
    }

    /// A file copied from the host that is DATA rather than a program.
    ///
    /// `0444` and not the executable mode: firmware is read by the kernel, and
    /// a file marked executable invites the question of what happens if
    /// something runs it. `ADR-008`'s answer is that nothing should be able to.
    fn data(
        path: impl Into<std::borrow::Cow<'static, str>>,
        source: PathBuf,
        reason: &'static str,
    ) -> Self {
        Self {
            path: path.into(),
            content: Content::CopiedFrom(source),
            mode: 0o444,
            uid: ROOT_UID,
            gid: ROOT_GID,
            reason,
        }
    }

    /// A file this build rendered. The mode comes from the definition rather
    /// than from a constant here, because `/etc/shadow` is `0600` and everything
    /// else is `0644` — see `rivet_core::etc::SHADOW_MODE`.
    fn generated(path: &'static str, bytes: Vec<u8>, mode: u32, reason: &'static str) -> Self {
        Self {
            path: path.into(),
            content: Content::Generated(bytes),
            mode,
            uid: ROOT_UID,
            gid: ROOT_GID,
            reason,
        }
    }

    pub fn is_directory(&self) -> bool {
        self.content == Content::Directory
    }
}

/// The built artifacts this tree is assembled around.
///
/// Paths, because `rivet-image` does not build Rivet — `cargo` does, for
/// `aarch64-unknown-linux-musl`, and the results are handed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The `rivet-initd` binary, destined for [`INIT_PATH`].
    pub init: PathBuf,
    /// The `rivet-sysd` binary, destined for [`SUPERVISOR_PATH`].
    pub supervisor: PathBuf,
    /// The `rivet` CLI, destined for [`CLI_PATH`]. **Also the login shell** —
    /// see [`LOGIN_SHELLS`].
    pub cli: PathBuf,
    /// The `rivet-netd` binary, destined for [`NETD_PATH`].
    pub netd: PathBuf,
    /// The `rivet-mdnsd` binary, destined for [`MDNSD_PATH`]. `PLAN.md` §23.
    pub mdnsd: PathBuf,
    /// The `git` binary, destined for [`GIT_PATH`]. `PLAN.md` §43.
    ///
    /// **Not built by this workspace**, and for `ADR-009` §6's reason rather
    /// than [`Inputs::nushell`]'s: it is a C mechanism, so it is Buildroot's.
    /// `tools/buildroot/build.sh` produces it at
    /// `target/usr/bin/git`, and **the one file is what is wanted** — see
    /// [`GIT_PATH`] for why the `git-core` directory beside it is not.
    pub git: PathBuf,
    /// The `git-http-backend` binary, destined for [`HTTP_BACKEND_PATH`].
    ///
    /// **Not built by this workspace and not the same file as
    /// [`Inputs::git`]** — Buildroot leaves it at
    /// `target/usr/libexec/git-core/git-http-backend`, which is a directory
    /// this build otherwise ignores entirely.
    pub http_backend: PathBuf,
    /// The `rivet-timed` binary, destined for [`TIMED_PATH`]. `ADR-021` §7.
    pub timed: PathBuf,
    /// The `rivet-deploy` binary, destined for [`DEPLOY_PATH`]. `ADR-029`
    /// §14.
    ///
    /// **Positional and required**, on [`Inputs::timed`]'s terms rather than
    /// [`Inputs::nushell`]'s. The optional inputs are optional because a card
    /// without them is a *supported* card (the ruling that makes `nu` optional, §7.4); a
    /// card that cannot install an update is not a variant, it is a card that
    /// can never be updated — and `ADR-018` §5 is the whole reason this
    /// project has two slots.
    pub deploy: PathBuf,
    /// The `rivet-mcpd` binary, destined for [`MCPD_PATH`] — **required**,
    /// for the reason that constant gives.
    pub mcpd: PathBuf,
    /// The `rivet-splash` binary, destined for [`SPLASH_PATH`] — **optional**.
    pub splash: Option<PathBuf>,
    /// The `rivet-bluetoothd` binary, destined for [`BLUETOOTHD_PATH`] —
    /// **optional**, and that constant says why it is not the splash's reason.
    pub bluetoothd: Option<PathBuf>,
    /// The `rivet-launcher` binary, destined for [`LAUNCHER_PATH`] —
    /// **optional**, and `ADR-050` §2 is why a card may carry it and not run
    /// it.
    pub launcher: Option<PathBuf>,
    /// The `rivet-displayd` binary, destined for [`DISPLAYD_PATH`] —
    /// **optional**, on the launcher's terms exactly. See that constant.
    pub displayd: Option<PathBuf>,
    /// The `rivet-proxyd` binary, destined for [`PROXYD_PATH`] — **optional**,
    /// on the launcher's terms. See that constant.
    pub proxyd: Option<PathBuf>,
    /// The `rivet-dnsd` binary, destined for [`DNSD_PATH`] — **optional**, on
    /// the proxy's terms. See that constant.
    pub dnsd: Option<PathBuf>,
    /// The `wpa_supplicant` binary, destined for [`SUPPLICANT_PATH`] -
    /// **optional**. Buildroot's, at `target/usr/sbin/wpa_supplicant`.
    pub supplicant: Option<PathBuf>,
    /// `wpa_cli`, destined for [`WPA_CLI_PATH`] - **optional**, and a bring-up
    /// tool rather than part of the machine. See the constant.
    pub wpa_cli: Option<PathBuf>,
    /// The `udhcpc` binary, destined for [`UDHCPC_PATH`] - **optional**.
    /// Buildroot leaves it at `target/bin/busybox`; see the constant for why it
    /// is installed under a different name.
    pub udhcpc: Option<PathBuf>,
    /// The `rivet-dhcp-hook` binary, destined for [`DHCP_HOOK_PATH`].
    ///
    /// **Optional on its own and REQUIRED beside `udhcpc`** — see
    /// [`SystemError::DhcpClientWithoutItsHook`]. Built by this workspace,
    /// unlike the DHCP client itself.
    pub dhcp_hook: Option<PathBuf>,
    /// The musl dynamic loader, destined for [`LOADER_PATH`] — **optional**.
    ///
    /// Buildroot's, at `host/<tuple>/sysroot/lib/libc.so`. On musl the loader
    /// and the C library are one file, so this is `libc.so` installed under the
    /// name a `PT_INTERP` names it by.
    ///
    /// **Optional, and a card without it is a supported card** — on
    /// [`Inputs::nushell`]'s terms rather than [`Inputs::deploy`]'s. Every Rivet
    /// binary is static (`ADR-003` §5), so an image with no loader boots,
    /// converges and is reachable exactly as every image before 2026-08-31 did.
    /// What it cannot do is run a dynamically-linked *application*, and the
    /// installer's closure check is what says so rather than a loader on the
    /// board.
    pub loader: Option<PathBuf>,
    /// **Vendor shared libraries, destined for `/lib`** — Mesa, 2026-08-31.
    ///
    /// Each is installed at `/lib/<file name>`, so the caller has already named
    /// them by their SONAME: `libEGL.so.1`, not `libEGL.so.1.0.0`. That is
    /// deliberate and it is what removes symlinks from this image entirely —
    /// a `NEEDED` entry names a SONAME, and a file installed under that name is
    /// found with no link to follow. `tools/card/build.sh` does the resolving,
    /// because it is the half that has `readelf` and a Buildroot tree.
    ///
    /// **`/lib` and not `/system/lib`, which is not a style choice.** musl's
    /// loader searches `/lib:/usr/local/lib:/usr/lib` with no configuration, so
    /// a library here is found by an application that declares nothing, sets no
    /// `LD_LIBRARY_PATH` and carries no `DT_RUNPATH`. `/system/lib` would work
    /// only by teaching `rivet-sysd` to inject a path into every service, which
    /// is a mechanism for a problem the default search path already solves.
    ///
    /// **It is still vendor content and still A/B.** `/lib` lives on the system
    /// partition beside [`LOADER_PATH`], so an update replaces these whole and
    /// in lockstep with the kernel — which matters more here than for most
    /// libraries, because Mesa's DRM interface has to match the kernel's.
    ///
    /// **Empty is the ordinary case.** A card with no libraries is every card
    /// built before Mesa, and boots identically.
    pub libraries: Vec<PathBuf>,
    /// **Shared objects at a path the caller names** — `(destination, source)`.
    ///
    /// [`Self::libraries`] installs into `/lib`, which is where the loader
    /// looks for a `NEEDED` entry. **A `dlopen`ed plugin is not a `NEEDED`
    /// entry**, and the library that opens it has the directory compiled in —
    /// `libgbm` carries the string `/usr/lib/gbm` and looks nowhere else. A
    /// plugin therefore has to land exactly where its loader was built to look,
    /// which only the caller knows.
    ///
    /// This is how Mesa's GBM backend (`dri_gbm.so`) reaches the image, and the
    /// reason it needed a second mechanism at all is worth stating: **a NEEDED
    /// closure cannot see a plugin.** `tools/card/build.sh` walks `NEEDED` and
    /// walked right past this one, producing a card whose libraries were all
    /// present and whose `gbm_create_device` returned NULL.
    pub libraries_at: Vec<(String, PathBuf)>,
    /// The DIRECTORY holding the CYW43455 firmware - Buildroot's
    /// `target/lib/firmware/brcm`. **Optional**, and a directory rather than
    /// three paths because [`FIRMWARE_FILES`] already knows the names and a
    /// caller repeating them is a caller that can get one wrong.
    pub wifi_firmware: Option<PathBuf>,

    /// The `femto` binary, destined for [`FEMTO_PATH`] — **optional**.
    ///
    /// `ADR-005` §5.5.1's editor. See [`FEMTO_PATH`] for why this is optional
    /// rather than positional; the short version is that a card without an
    /// editor is a supported card.
    pub femto: Option<PathBuf>,

    /// The CA bundle, destined for `rivet_core::etc::CA_BUNDLE_SOURCE` —
    /// **optional**, and its absence is a machine that cannot verify TLS.
    ///
    /// Trust roots are IMAGE content: they are replaced by an A/B update and
    /// nothing on a running machine can change what it trusts. The supervisor
    /// projects this file into `/etc/ssl/certs/ca-certificates.crt`, where Go
    /// and OpenSSL look — see `rivet_core::etc::CA_BUNDLE` for why a file
    /// shipped at that path directly would be masked by the /etc tmpfs and
    /// achieve nothing.
    pub ca_bundle: Option<PathBuf>,

    /// The system typeface, destined for `rivet_core::etc::SYSTEM_FONT` —
    /// **optional**, and its absence is a machine whose screens cannot draw
    /// a word. `PLAN.md` §104.38.
    pub system_font: Option<PathBuf>,

    /// The system monospace, destined for `rivet_core::etc::SYSTEM_MONO` —
    /// optional, and its absence is a machine whose changing numbers shift
    /// sideways under themselves. Added 2026-09-04.
    pub mono_font: Option<PathBuf>,

    /// The icon face, destined for `rivet_core::etc::ICON_FONT` — optional, and
    /// its absence is a machine whose screens have words and no symbols.
    pub icon_font: Option<PathBuf>,

    /// **The zone database**, destined for `rivet_core::etc::ZONEINFO_DIR` —
    /// optional, and its absence is a machine that can only keep UTC.
    /// `ADR-021` §9, 2026-09-02.
    ///
    /// Each entry is `(name, source)`: the IANA identifier as it will be
    /// looked up — `America/Chicago`, with forward slashes on every host — and
    /// the file to copy. **The caller enumerates**, not [`tree`], for the
    /// reason [`tree`] is a pure function of its inputs: walking a directory is
    /// I/O, and `cmd/rivet-image`'s `main` is where the I/O already is.
    ///
    /// A `Vec` rather than a directory path is also what lets the caller state
    /// *which* files it took and why — see `main`'s `--zoneinfo`, which ships
    /// only files that actually begin `TZif` and refuses the `posix/` and
    /// `right/` subtrees by name.
    pub zoneinfo: Vec<(String, PathBuf)>,
    /// The `nu` binary, destined for [`NUSHELL_PATH`] — **optional**.
    ///
    /// a card built without it is a supported card,
    /// not a broken one. `None` produces an image whose only login shell is the
    /// CLI, which is every card built before 2026-08-24 and is the state the
    /// runtime projection is required to keep handling.
    ///
    /// **Not built by this workspace**, and NOT for [`Inputs::git`]'s reason,
    /// which is worth not conflating. `git` is a C mechanism, so it is
    /// Buildroot's (`ADR-009` §6). `nu` is a *Rust* program, and
    /// it is an input here only because it needs a C cross-toolchain of its own
    /// (`ADR-005` §5.6.3: `fff-search` drags in `libgit2-sys`, LMDB and zlib,
    /// and no feature combination avoids it). `tools/nushell/build.sh` produces
    /// it.
    pub nushell: Option<PathBuf>,
    /// The `gdbserver` binary, destined for [`GDBSERVER_PATH`] — **optional**.
    ///
    /// **Not built by this workspace**, and for [`Inputs::git`]'s reason rather
    /// than `nu`'s: it is a C mechanism, so it is Buildroot's (`ADR-009` §6).
    /// `tools/buildroot/build.sh gdb` leaves it at `target/usr/bin/gdbserver`.
    ///
    /// Optional because `ADR-005` §7.4 is undecided, not because a card without
    /// it is degraded — see [`GDBSERVER_PATH`].
    pub gdbserver: Option<PathBuf>,
    /// The `strace` binary, destined for [`STRACE_PATH`] — **optional**, on the
    /// same terms as [`Inputs::gdbserver`]. `tools/buildroot/build.sh strace`
    /// leaves it at `target/usr/bin/strace`.
    pub strace: Option<PathBuf>,
    /// What this image trusts, as `ADR-028` §5.5's entries.
    ///
    /// **Empty is legal here and refused by [`tree`]** — see
    /// [`SystemError::NoTrustedKeys`]. The type does not forbid it because
    /// `--no-trust` is a real, supported, loudly-marked choice; what is not
    /// supported is arriving at an untrusting image by omission.
    pub trust: TrustStore,

    /// **What this image's binaries execute on**, as `ADR-027` §3's `<arch>`
    /// token — written to [`ARCH_PATH`].
    ///
    /// Added 2026-09-12 (`PLAN-Feature-Set-2026-09-12` §3). The store's door
    /// and the installer compare every artifact's header against it, and it has
    /// to come from here rather than from the compiler on the machine:
    /// `std::env::consts::ARCH` says `arm` where `ADR-027` says `armv6`. The card
    /// build chose the target triple every binary was compiled for, so it is the
    /// one party that knows the token. **A `String` and not an `Option`**: an
    /// image with no record admits no artifact, so omitting it is not a smaller
    /// card but a broken one, and [`tree`] refuses a token that is not one.
    pub arch: String,

    /// **Deliberately produce an image that trusts nobody.**
    ///
    /// `ADR-028` §5.5.5: an image with no trusted key cannot verify an update,
    /// and therefore cannot be updated safely. That is a legitimate thing to
    /// build — a bring-up card, a test image — and an illegitimate thing to do
    /// by accident, so it takes a word.
    pub development_only: bool,

    /// Which of this image's binaries the runtime projection may name as a
    /// login shell.
    ///
    /// `main` passes [`LOGIN_SHELLS`]; see that constant for why the value is
    /// both a constant and a field, and for why the guard it feeds asks "at
    /// least one" rather than "this one".
    pub login_shells: &'static [&'static str],
}

/// What can be wrong with the inputs. Refused rather than repaired —
/// `AGENTS.md` §5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemError {
    /// A source path that is not absolute resolves against the build process's
    /// working directory, so the same command would produce different images
    /// from different directories. `ADR-015` §4 signs these.
    SourceNotAbsolute {
        /// Which input — the destination path, which names it unambiguously.
        role: &'static str,
        path: PathBuf,
    },
    /// The image would trust nobody, and `--no-trust` was not passed.
    ///
    /// `ADR-028` §5.5.5, ruled 2026-08-24. An image with an empty trust store
    /// cannot verify an update, so it cannot be updated safely — and that is
    /// not discovered at build time unless something refuses here. It is
    /// discovered on a board, later, by somebody trying to fix something else.
    /// `AGENTS.md` §3: refuse rather than produce a thing that looks finished.
    NoTrustedKeys,

    /// A card carrying the DHCP client and not the program it execs.
    ///
    /// **This is the failure that would otherwise be silent.** `udhcpc` would
    /// run, broadcast, receive an offer, accept it — and then exec a path that
    /// is not there. Nothing would set an address, and the machine would look
    /// like one whose network was simply down. `AGENTS.md` §3: refuse rather
    /// than produce a thing that looks finished.
    ///
    /// The reverse is NOT refused: a hook with no client is inert and harmless,
    /// and it is what a card built before the client existed looks like.
    DhcpClientWithoutItsHook,

    /// The trust store could not be rendered — a label that will not fit, or
    /// more kinds than a record holds.
    TrustStoreRefused(String),

    /// **Not one** of [`LOGIN_SHELLS`] is in this image.
    ///
    /// The card would boot, the runtime projection would render an
    /// `/etc/shells` naming nothing, and every login would be refused
    /// *before* authentication — a failure that lands on a board rather than on
    /// the build machine and that names neither this constant nor
    /// `/etc/shells`. Measured on real hardware: that class of thing
    /// costs a long time to diagnose.
    ///
    /// **Renamed from `LoginShellNotInImage` on 2026-08-24**, when
    /// the dual-login-shell ruling made the shells a list. The old name described a stricter check
    /// — *the* named shell must be present — which a candidate list makes
    /// wrong: a missing `nu` is an ordinary card, and refusing to build it
    /// would be the guard enforcing a decision nobody took.
    NoLoginShellInImage { candidates: Vec<String> },

    /// The architecture this image was told it is built for is not one of
    /// `ADR-027` §3's tokens. Refused here, because every machine written from
    /// this image would refuse every artifact against a record it cannot read —
    /// and it would say so on a board rather than on the build machine.
    UnknownArch(String),
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotAbsolute { role, path } => write!(
                f,
                "the source for {role} is not an absolute path: {}",
                path.display()
            ),
            Self::NoTrustedKeys => write!(
                f,
                "REFUSED this image would trust no signing key, so nothing on it could ever verify an update — and a machine that cannot verify an update cannot be updated safely (ADR-028 §5.5.5). Pass --trusted-key <path> with the public key that may sign for this fleet. If you genuinely want a card that trusts nobody — a bring-up image, a test card — pass --no-trust and the image will say so on every boot."
            ),
            Self::DhcpClientWithoutItsHook => write!(
                f,
                "REFUSED this image carries the DHCP client and not the program it execs. udhcpc would run, broadcast, take an offer, and then exec {DHCP_HOOK_PATH} — which is not on this card. Nothing would set an address and the machine would look like one whose network is simply down, which is the failure this refusal exists to make loud. Pass --dhcp-hook <path to rivet-dhcp-hook>, or drop --udhcpc for a card with no DHCP."
            ),
            Self::TrustStoreRefused(why) => {
                write!(f, "the trust store could not be written: {why}")
            }
            Self::NoLoginShellInImage { candidates } => write!(
                f,
                "not one of LOGIN_SHELLS is in this image ({}). The runtime projection would \
                 render an /etc/shells naming nothing, and an account whose shell is unlisted \
                 is refused before any credential is looked at — so \
                 nothing could log in to the card this build would produce",
                candidates.join(", ")
            ),
            Self::UnknownArch(found) => write!(
                f,
                "REFUSED --arch {found:?} is not one of ADR-027 §3's architecture tokens ({}). \
                 The image records it at {ARCH_PATH}, and the machine compares every artifact \
                 against it — a token it cannot read would make that machine refuse everything. \
                 It is the token for the target triple the binaries were built for: \
                 aarch64-unknown-linux-musl is aarch64, arm-unknown-linux-musleabihf is armv6.",
                rivet_core::artifact::ARCH_TOKENS.join(", ")
            ),
        }
    }
}

impl std::error::Error for SystemError {}

/// The complete tree, sorted by path.
///
/// The sort is the interface, not a tidiness: a path always sorts after every
/// one of its ancestors, so a caller may create the entries in this order and
/// never need to think about parents. `the_order_creates_a_parent_before_its_children`
/// is what keeps that true.
///
/// It is the **same tree for both slots**. Nothing inside `/system` knows which
/// slot it is; what differs is `cmdline.txt`'s `root=` on the boot partition,
/// which is [`boot`](crate::boot)'s and [`layout::Slot`](crate::layout::Slot)'s.
/// So there is no `Slot` parameter, and a parameter that was ignored would be
/// worse than none.
pub fn tree(inputs: &Inputs) -> Result<Vec<Node>, SystemError> {
    // `has_root` rather than `is_absolute`, deliberately. They are the same
    // predicate on Linux, where this tool runs; they differ on Windows, where
    // `/build/…` is *not* absolute — and this crate is edited from a Windows
    // host, so `cargo test` there would otherwise fail on a path that is
    // perfectly good for the machine the image is built on.
    //
    // Checked in the order the binaries appear in [`Inputs`], as a loop rather
    // than as four copies of the same three lines: the copies were the shape
    // this had at two, and a fourth binary would have been the point at which
    // one of them was forgotten.
    let mut sources = vec![
        (INIT_PATH, &inputs.init),
        (SUPERVISOR_PATH, &inputs.supervisor),
        (CLI_PATH, &inputs.cli),
        (NETD_PATH, &inputs.netd),
        (MDNSD_PATH, &inputs.mdnsd),
        (GIT_PATH, &inputs.git),
        (HTTP_BACKEND_PATH, &inputs.http_backend),
        (TIMED_PATH, &inputs.timed),
        (DEPLOY_PATH, &inputs.deploy),
    ];
    // The optional one is checked by the same loop rather than beside it. A
    // relative `--nushell` path is the identical defect as a relative `--cli`
    // one — the same command builds a different image from a different
    // directory — and optionality is about whether the argument was given, not
    // about whether it has to be correct.
    sources.push((MCPD_PATH, &inputs.mcpd));
    if let Some(bluetoothd) = &inputs.bluetoothd {
        sources.push((BLUETOOTHD_PATH, bluetoothd));
    }
    if let Some(launcher) = &inputs.launcher {
        sources.push((LAUNCHER_PATH, launcher));
    }
    if let Some(displayd) = &inputs.displayd {
        sources.push((DISPLAYD_PATH, displayd));
    }
    if let Some(proxyd) = &inputs.proxyd {
        sources.push((PROXYD_PATH, proxyd));
    }
    if let Some(dnsd) = &inputs.dnsd {
        sources.push((DNSD_PATH, dnsd));
    }
    if let Some(splash) = &inputs.splash {
        sources.push((SPLASH_PATH, splash));
    }
    if let Some(supplicant) = &inputs.supplicant {
        sources.push((SUPPLICANT_PATH, supplicant));
    }
    if let Some(wpa_cli) = &inputs.wpa_cli {
        sources.push((WPA_CLI_PATH, wpa_cli));
    }
    if let Some(udhcpc) = &inputs.udhcpc {
        sources.push((UDHCPC_PATH, udhcpc));
    }
    if let Some(hook) = &inputs.dhcp_hook {
        sources.push((DHCP_HOOK_PATH, hook));
    }
    if let Some(femto) = &inputs.femto {
        sources.push((FEMTO_PATH, femto));
    }
    if let Some(nu) = &inputs.nushell {
        sources.push((NUSHELL_PATH, nu));
    }
    // The debugging mechanisms, on identical terms — ADR-005 §7. Optional about
    // whether they were *given*, never about whether the path is usable.
    if let Some(gdbserver) = &inputs.gdbserver {
        sources.push((GDBSERVER_PATH, gdbserver));
    }
    if let Some(strace) = &inputs.strace {
        sources.push((STRACE_PATH, strace));
    }
    for (role, path) in sources {
        if !path.has_root() {
            return Err(SystemError::SourceNotAbsolute {
                role,
                path: path.clone(),
            });
        }
    }

    // `ADR-028` §5.5.5, ruled 2026-08-24. An image that trusts nobody cannot
    // verify an update and therefore cannot be updated safely. Building one is
    // legitimate — a bring-up card, a test image — and doing so *by omission*
    // is not, so it costs a word on the command line.
    // **A DHCP client with nothing to exec is the SILENT failure**, so it is
    // refused here rather than found on a board: udhcpc would run, take an
    // offer, and exec a path that is not there. Not the reverse -- a hook with
    // no client is inert, and is what a card built before the client looks like.
    if inputs.udhcpc.is_some() && inputs.dhcp_hook.is_none() {
        return Err(SystemError::DhcpClientWithoutItsHook);
    }

    if inputs.trust.is_empty() && !inputs.development_only {
        return Err(SystemError::NoTrustedKeys);
    }
    let trust_bytes = inputs
        .trust
        .to_bytes()
        .map_err(|e| SystemError::TrustStoreRefused(e.to_string()))?;
    let arch_bytes = rivet_core::package::arch::render(&inputs.arch)
        .ok_or_else(|| SystemError::UnknownArch(inputs.arch.clone()))?;

    let mut nodes = vec![
        Node::dir(
            ROOT,
            "the image root; its mode is stated rather than inherited from the builder's umask",
        ),
        // `/dev` is NOT listed here any more, and its absence is the fix rather
        // than an omission. It was a hand-written node because `stage0_plan`
        // did not mount it; since 2026-08-24 the plan does, and the
        // plan-driven pass below adds every `Provision::InImage` target. Both
        // ran, so the tree carried `/dev` twice — caught by
        // `the_tree_is_sorted_and_holds_no_path_twice`, which is exactly the
        // drift it was written for. One authority (`ADR-011`): the plan.
        Node::dir(
            ETC,
            "musl's compiled-in address for the mechanisms' formats — ADR-008 §5.1",
        ),
        Node::dir(
            STATE_MOUNT_POINT,
            "ADR-008 §3's persistent partition; the mount point cannot be made later",
        ),
        Node::dir(
            "/system",
            "ADR-008 §3: the image itself — read-only and replaced wholesale",
        ),
        Node::dir(
            "/system/bin",
            "ADR-016 §3 names it the base image's binaries",
        ),
        Node::generated(
            TRUST_STORE_PATH,
            trust_bytes,
            0o444,
            "what this machine trusts, and for which kinds — ADR-028 §5.5",
        ),
        Node::generated(
            ARCH_PATH,
            arch_bytes,
            rivet_core::package::arch::MODE,
            "what this machine executes, which every artifact is compared against — ADR-027 §3",
        ),
        Node::executable(
            INIT_PATH,
            inputs.init.clone(),
            "PID 1, and what cmdline.txt's init= names",
        ),
        Node::executable(
            SUPERVISOR_PATH,
            inputs.supervisor.clone(),
            "PID 1's single child — ADR-017 §3.1",
        ),
        Node::executable(
            CLI_PATH,
            inputs.cli.clone(),
            "the CLI, and the login shell /etc/shells names",
        ),
        Node::executable(
            NETD_PATH,
            inputs.netd.clone(),
            "Stage 1's wired bring-up — ADR-014 §2; the supervisor's \
             system_services starts it",
        ),
        Node::executable(
            MDNSD_PATH,
            inputs.mdnsd.clone(),
            "the mDNS responder — PLAN.md §23; the supervisor's definition names this path and \
             ships it DISABLED, so a card carries it and does not run it until the document \
             says so",
        ),
        Node::executable(
            GIT_PATH,
            inputs.git.clone(),
            "git — ADR-009 §6 Tier B; ONE binary and not the 171 entries of usr/libexec/git-core, \
             because every verb Rivet needs is a builtin reached as `git <verb>` (PLAN.md §43.6)",
        ),
        Node::executable(
            HTTP_BACKEND_PATH,
            inputs.http_backend.clone(),
            "git-http-backend - git's own smart-HTTP server, ADR-035 §4. A SECOND binary and not \
             a builtin (PLAN.md §84.1), and it needs /system/bin on PATH: it runs `git` rather \
             than reaching upload-pack by --exec-path, and with no PATH it serves an EMPTY              repository instead of failing (PLAN.md §85.2)",
        ),
        Node::executable(
            TIMED_PATH,
            inputs.timed.clone(),
            "the SNTP client — ADR-021 §7; the supervisor's definition names this path",
        ),
        Node::executable(
            DEPLOY_PATH,
            inputs.deploy.clone(),
            "the installer — ADR-029 §14; NOTHING ON THE CARD STARTS IT, because how a \
             payload reaches it is not ruled (the maintainer's 2026-08-24 install-request ruling)",
        ),
        Node::dir(
            EXT_MOUNT_POINT,
            "ADR-018 §6: a tmpfs goes over this, and it cannot be created on a read-only filesystem",
        ),
    ];

    // rivet-mcpd — ADR-035 phase 1, installed 2026-08-28 and REQUIRED since the
    // supervisor began defining the service the same day. Pushed rather than
    // declared for nushell's reason below.
    nodes.push(Node::executable(
        MCPD_PATH,
        inputs.mcpd.clone(),
        "the control plane's listener — ADR-035. THE administration surface: cmd/rivet-sysd \
             defines the service, ships it ENABLED, and REACHABILITY_SET refuses to let a reload \
             stop it, because a board without this cannot be reached at all (ADR-017 §6.2.1)",
    ));

    // The boot splash, when the build was given one. **PID 1 runs this**, which
    // is the difference between it and femto below: an absent splash is a
    // console-looking boot rather than a missing convenience.
    if let Some(splash) = &inputs.splash {
        nodes.push(Node::executable(
            SPLASH_PATH,
            splash.clone(),
            "the boot splash - drawn once on /dev/fb0 by pid 1, on an ordinary boot and NOT on a \
             trial boot, so a trial shows the console it is being watched for",
        ));
    }

    // The BLE observer, when the build was given one. It is paired with the
    // KERNEL rather than with a screen - see BLUETOOTHD_PATH.
    if let Some(bluetoothd) = &inputs.bluetoothd {
        nodes.push(Node::executable(
            BLUETOOTHD_PATH,
            bluetoothd.clone(),
            "the BLE observer - ADR-041, the arbiter of the one Bluetooth adapter. The \
             supervisor's definition names this path and ships it DISABLED, and the binary is \
             useless on a kernel built before 2026-09-05 because CONFIG_BT was a module and a \
             Rivet loads none",
        ));
    }

    // The kiosk menu, when the build was given one — ADR-050 §2's tier 2.
    // The supervisor's definition names this path and ships it DISABLED, so a
    // headless card carrying it behaves exactly as a card without it.
    if let Some(launcher) = &inputs.launcher {
        nodes.push(Node::executable(
            LAUNCHER_PATH,
            launcher.clone(),
            "the kiosk menu - ADR-050 2.1, the program that holds DRM master and offers the \
             applications this machine has. The supervisor's definition names this path and \
             ships it DISABLED, so a card carrying it shows nothing until the document enables \
             it - which is what 'headless leaves launcher off, kiosk adds launcher' means",
        ));
    }

    // The compositor, when the build was given one — ADR-058 §4. Tier 2 on
    // the launcher's terms: the definition names this path and ships it
    // DISABLED.
    if let Some(displayd) = &inputs.displayd {
        nodes.push(Node::executable(
            DISPLAYD_PATH,
            displayd.clone(),
            "the compositor - ADR-058 4.1, the program that owns the GPU and KMS and hands an \
             application a SURFACE rather than a DRM lease. It is the one binary of Rivet's own \
             that is DYNAMICALLY LINKED (ADR-058 5.1: Mesa stages no static libgbm), so a card \
             carrying it must also carry libgbm.so.1, libxkbcommon.so.0 and libgcc_s.so.1 - \
             which tools/card/build.sh's must_be_runnable checks before anything is written",
        ));
    }

    // The path proxy, when the build was given one — ADR-060. Tier 2 on the
    // launcher's terms: the definition names this path and ships it DISABLED,
    // and it runs under its own uid and ruleset rather than as root.
    if let Some(proxyd) = &inputs.proxyd {
        nodes.push(Node::executable(
            PROXYD_PATH,
            proxyd.clone(),
            "the path proxy - ADR-060, the machine's front door on 443, part of the operating \
             system and run in user space under its own uid and Landlock ruleset. The \
             supervisor's definition names this path and ships it DISABLED, so it serves nothing \
             until the document's role enables it",
        ));
    }

    // The name service, when the build was given one — ADR-047. Shipped
    // DISABLED; a document enables it.
    if let Some(dnsd) = &inputs.dnsd {
        nodes.push(Node::executable(
            DNSD_PATH,
            dnsd.clone(),
            "the name service - ADR-047, answering for the fleet's home.arpa names from the \
             document's [dns] table and forwarding every other query upstream. The supervisor's \
             definition names this path and ships it DISABLED",
        ));
    }

    // Wireless, when the build was given it - ADR-004, installed 2026-08-30.
    // Three independent options rather than one: a card may reasonably carry
    // the supplicant and not `wpa_cli`, and the firmware is useful on its own -
    // it is what makes `wlan0` APPEAR, which is the first thing to prove.
    if let Some(supplicant) = &inputs.supplicant {
        nodes.push(Node::executable(
            SUPPLICANT_PATH,
            supplicant.clone(),
            "the WPA supplicant - ADR-004 §3's PERMANENT Tier C dependency and not a stopgap:              depending on it is what keeps Rivet on WPA3-SAE and whatever follows at no ongoing \
             cost. rivet-netd drives its control SOCKET (§4), never an FFI linkage",
        ));
    }
    if let Some(wpa_cli) = &inputs.wpa_cli {
        nodes.push(Node::executable(
            WPA_CLI_PATH,
            wpa_cli.clone(),
            "wpa_cli - a BRING-UP tool rather than part of the machine: it proves the C side \
             associates independently of Rivet's own client, so a failure has one candidate \
             cause instead of two. Nothing on the card runs it",
        ));
    }
    if let Some(udhcpc) = &inputs.udhcpc {
        nodes.push(Node::executable(
            UDHCPC_PATH,
            udhcpc.clone(),
            "the DHCP client - BusyBox udhcpc, built with exactly ONE applet. It configures NOTHING: it speaks DHCP and execs a program with the lease in the environment, so \
             rivet-netd keeps every netlink write and ADR-011's one authority survives",
        ));
    }
    if let Some(hook) = &inputs.dhcp_hook {
        nodes.push(Node::executable(
            DHCP_HOOK_PATH,
            hook.clone(),
            "what udhcpc execs when a lease changes - it writes the lease to /run and performs \
             NO netlink operation, so rivet-netd stays the only thing that sets an address. This \
             path is compiled into BusyBox as CONFIG_UDHCPC_DEFAULT_SCRIPT",
        ));
    }
    // **The dynamic loader** — Shared Rivets, 2026-08-31. See [`LOADER_PATH`].
    //
    // `Node::executable` rather than `Node::data`: the kernel `execve`s this
    // path when it starts a dynamically-linked program, so it is a program in
    // the only sense that matters here — and it is directly runnable besides
    // (`ld-musl-aarch64.so.1 --list <binary>` is musl's `ldd`).
    //
    // **`/lib` itself is not created here**, and that is not an omission: the
    // erofs builder creates a node's parents, and `/lib/firmware/brcm` below
    // has been relying on the same thing since 2026-08-30. Stated because the
    // two entries look like they should be symmetrical and one has an explicit
    // `Node::dir` while the other does not.
    if let Some(loader) = &inputs.loader {
        nodes.push(Node::executable(
            LOADER_PATH,
            loader.clone(),
            "musl's dynamic loader, at the path the cross toolchain writes into every \
             dynamically-linked binary's PT_INTERP. NOTHING IN THIS IMAGE USES IT — every \
             Rivet binary is static (ADR-003 §5). It is here so an APPLICATION can be \
             dynamic, which is what Shared Rivets needs and the only thing shipping a \
             loader turned out to mean on musl",
        ));
    }

    // **Vendor shared libraries** — Mesa, 2026-08-31. Beside the loader, in
    // the one directory musl searches without being told to.
    //
    // `Node::executable` for the same reason the loader is one: a shared object
    // carries executable code and the loader maps it that way. `0644` would
    // work on Linux and would be a lie about what the file is.
    for library in &inputs.libraries {
        let name = library
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        nodes.push(Node::executable(
            format!("{LIB_DIR}/{name}"),
            library.clone(),
            "a vendor shared library, at the SONAME an application's NEEDED entry names it \
             by. musl searches /lib with no configuration, so nothing declares this and \
             nothing sets LD_LIBRARY_PATH for it",
        ));
    }

    // **Not in `sources`**, which is the list of executables: this is data, at
    // `Node::data`'s 0444, and it needs a directory that nothing else creates.
    // **The directory comes first, and it comes from ALL THREE faces rather
    // than from the sans alone.** Restructured 2026-09-04 when the monospace
    // was added.
    //
    // It used to be pushed inside the `system_font` branch, and AFTER the icon
    // face's file node. So an image built with an icon face and no sans put a
    // file in a directory that was never created — a shape nothing had built
    // because `tools/card/build.sh` passes both or neither, which is exactly
    // the sort of latent hole a third face turns into a real one.
    if inputs.system_font.is_some() || inputs.mono_font.is_some() || inputs.icon_font.is_some() {
        nodes.push(Node::dir(
            FONT_DIR,
            "the typefaces this image carries; nothing else puts a file here",
        ));
    }
    if let Some(font) = &inputs.icon_font {
        nodes.push(Node::data(
            rivet_core::etc::ICON_FONT,
            font.clone(),
            "the icon face every Rivet screen draws symbols from. Glyphs rather than images, so \
             nothing on this machine needs an image decoder",
        ));
    }
    if let Some(font) = &inputs.system_font {
        nodes.push(Node::data(
            rivet_core::etc::SYSTEM_FONT,
            font.clone(),
            "the typeface every Rivet screen draws with. An image without it produces a \
             launcher that can be navigated blind and a splash with no words on it",
        ));
    }
    if let Some(font) = &inputs.mono_font {
        nodes.push(Node::data(
            rivet_core::etc::SYSTEM_MONO,
            font.clone(),
            "the monospace, for the numbers that change in place. An image without it draws a \
             clock whose digits shift sideways every second, which is legible and looks broken",
        ));
    }
    // **The zone database** — `ADR-021` §9, 2026-09-02. Data at `Node::data`'s
    // 0444, in a directory nothing else puts a file in.
    //
    // ~600 nodes on a full database, which is more entries than the rest of
    // this tree put together and is still the right shape: each is a file the
    // image carries, at a path, with a mode, and the erofs builder has no
    // cheaper way to be told about a directory. The alternative — a tarball or
    // a squashfs the running machine mounts — would put a second filesystem in
    // the image to save a list.
    if !inputs.zoneinfo.is_empty() {
        nodes.push(Node::dir(
            rivet_core::etc::ZONEINFO_DIR,
            "the zone database the /etc/localtime projection reads; nothing else puts a file \
             here",
        ));
        for (name, source) in &inputs.zoneinfo {
            nodes.push(Node::data(
                format!("{}/{name}", rivet_core::etc::ZONEINFO_DIR),
                source.clone(),
                "one IANA zone, as a tzfile. The supervisor copies the one the document names \
                 to /etc/localtime, where musl, Go and chrono all read it; a machine whose \
                 zone is not here keeps UTC and says so",
            ));
        }
    }

    if let Some(bundle) = &inputs.ca_bundle {
        nodes.push(Node::dir(
            CA_BUNDLE_DIR,
            "the trust roots this image ships; nothing else puts a file here",
        ));
        nodes.push(Node::data(
            rivet_core::etc::CA_BUNDLE_SOURCE,
            bundle.clone(),
            "the CA certificates every TLS client on this machine verifies against. The \
             supervisor projects it into /etc/ssl/certs, where Go and OpenSSL look; an image \
             without it produces `certificate signed by unknown authority` for every HTTPS \
             request and nothing else explains why",
        ));
    }
    // Plugins, at the paths their loaders have compiled in. See
    // [`Inputs::libraries_at`].
    for (destination, source) in &inputs.libraries_at {
        nodes.push(Node::executable(
            destination.clone(),
            source.clone(),
            "a shared object at a path its loader has COMPILED IN rather than searched for. \
             A dlopened plugin is invisible to a NEEDED closure, so nothing computes this \
             one - the caller names it",
        ));
    }

    if let Some(dir) = &inputs.wifi_firmware {
        nodes.push(Node::dir(
            FIRMWARE_BRCM_DIR,
            "where the kernel's firmware loader looks - ADR-008 §5.1's one genuine \
             kernel-side path, compiled into the kernel rather than chosen",
        ));
        for (name, destination) in FIRMWARE_FILES {
            nodes.push(Node::data(
                *destination,
                dir.join(name),
                "the CYW43455's firmware, under the name ADR-004 §8.1 MEASURED the driver \
                 asking for. Buildroot installs these as SYMLINKS into cypress/; they land here \
                 as real files because a copied node is read with fs::read, which follows it",
            ));
        }
    }

    // femto, when the build was given one — ADR-005 §5.5.1, installed
    // 2026-08-24. Pushed on nushell's terms below, and for its reason.
    if let Some(femto) = &inputs.femto {
        nodes.push(Node::executable(
            FEMTO_PATH,
            femto.clone(),
            "the text editor — ADR-005 §5.5.1; an operator runs it, nothing on the card does",
        ));
    }

    // nushell, when the build was given one — ADR-005 §5.1, installed 2026-08-24.
    //
    // It is pushed rather than declared in the vec above because it is the one
    // entry that may legitimately be absent, and a `Vec<Option<Node>>` filtered
    // afterwards would make every other entry look conditional too.
    if let Some(nu) = &inputs.nushell {
        nodes.push(Node::executable(
            NUSHELL_PATH,
            nu.clone(),
            "the interactive login shell — ADR-005 §5.1; the projection probes for this path",
        ));
    }

    // The debugging mechanisms — ADR-005 §7, installed 2026-08-24. Nothing on
    // the card starts either: they are run by an operator who has logged in,
    // which is the whole difference between a debugger and a service.
    if let Some(gdbserver) = &inputs.gdbserver {
        nodes.push(Node::executable(
            GDBSERVER_PATH,
            gdbserver.clone(),
            "the remote debug stub — ADR-005 §7; an operator runs it, nothing on the card does",
        ));
    }
    if let Some(strace) = &inputs.strace {
        nodes.push(Node::executable(
            STRACE_PATH,
            strace.clone(),
            "the syscall tracer — ADR-005 §7.2; the only way to read a mechanism's errno",
        ));
    }

    // The mount points PID 1 cannot create for itself, derived from the plan
    // rather than listed. See [`provision`] — this is the half of the drift
    // problem that can be solved by construction instead of by a test.
    for spec in stage0_plan() {
        if provision(spec.target) == Provision::InImage {
            nodes.push(Node::dir(
                spec.target,
                "a stage 0 mount point, on a filesystem PID 1 cannot mkdir into",
            ));
        }
    }

    // `/etc` ships EMPTY, and that is a ruling rather than an omission.
    //
    // **Ruled by the maintainer 2026-08-23** (`ADR-007` §3.5, `ADR-008` §5.3).
    // Between 2026-08-23 and the same day, this loop rendered five files here
    // from `rivet_core::etc` — `passwd`, `group`, `shadow`, `hosts`,
    // `shells` — on the argument that each is a fact about the *image* rather
    // than about the machine, and so could be known at build time.
    //
    // The argument was true and not sufficient. Nothing about those five
    // *requires* build-time generation: the runtime projection renders `/etc`
    // before anything that reads it starts, so it can render all of `/etc`,
    // and then there is **one** generator instead of two. Two generators over
    // one directory is the shape `ADR-011` exists to prevent, and defending it
    // as "disjoint facts" is a bill that comes due the first time a fact stops
    // being obviously one or the other.
    //
    // `rivet_core::etc` is kept in full — the definitions, the nine musl
    // paths and the argument for each file are the expensive part and the
    // runtime projection is what will render them. What was deleted is this
    // *second* renderer, not the knowledge.
    //
    // The cost is stated rather than hidden: without a runtime projection,
    // `/etc` is empty on a running Rivet, so every login is rejected before
    // authentication. Stage 1 cannot finish without that
    // projection. It could not finish without a login shell either, and —
    // 2026-08-23 — there now IS one: `rivet` is in this tree and `LOGIN_SHELL`
    // names it. `cmd/rivet-sysd`'s `etc` module is the projection, so what is
    // left here is nothing.
    check_login_shells(&nodes, inputs.login_shells)?;

    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(nodes)
}

/// Refuse an image that contains **none** of its candidate login shells.
///
/// A function taking the value rather than reading the constant, so that both
/// arms can be driven by a test. `AGENTS.md` §4: a guard that cannot be
/// exercised proves nothing, and this one is on the accepting arm for every
/// real build.
///
/// **"At least one", not "each one", and the difference is the ruling that
/// makes `nu` optional.** The candidates are what the runtime projection will *probe* for;
/// it takes whichever it finds and is required to keep working when it finds
/// only some. Demanding all of them would make an image without `nu`
/// unbuildable, which is a decision this guard has no business taking. What it
/// still refuses is the thing it was written to refuse: a card nothing can log
/// in to.
///
/// An entry that is present but not executable does not count, because
/// `/etc/shells` naming it would pass `getusershell()` and then fail at `exec`
/// — the failure that lands on a board rather than here.
pub fn check_login_shells(nodes: &[Node], candidates: &[&str]) -> Result<(), SystemError> {
    if candidates.is_empty() {
        return Ok(());
    }
    let present = candidates.iter().any(|shell| {
        nodes
            .iter()
            .any(|n| n.path == *shell && !n.is_directory() && n.mode & 0o111 != 0)
    });
    if present {
        Ok(())
    } else {
        Err(SystemError::NoLoginShellInImage {
            candidates: candidates.iter().map(|s| (*s).to_string()).collect(),
        })
    }
}

/// Look one entry up. Convenience for callers and for tests.
pub fn find<'a>(tree: &'a [Node], path: &str) -> Option<&'a Node> {
    tree.iter().find(|n| n.path == path)
}

// ---------------------------------------------------------------------------
// Where a mount point comes from
// ---------------------------------------------------------------------------

/// Whether a mount point has to be in the image, and if not, what supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provision {
    /// The `mkdir` at runtime would land on this read-only filesystem, so the
    /// directory must already exist in it.
    InImage,
    /// Something is mounted over an ancestor before this target is reached, so
    /// the `mkdir` lands on *that* filesystem, which is writable. Putting the
    /// directory in the image would achieve nothing and hide the truth.
    UnderRuntimeMount { covered_by: &'static str },
}

/// Classify a path by *where a `mkdir` for it would land at runtime*.
///
/// The rule, and it is the whole of the reasoning: a path is supplied at
/// runtime if some ancestor of it is a filesystem that is mounted **before**
/// the path is needed. Two things can do that — the kernel's devtmpfs on
/// [`DEV_MOUNT_POINT`], and any earlier entry in [`stage0_plan`], whose order
/// `rivet-core`'s `stage0_plan_mounts_a_parent_before_anything_inside_it`
/// already guarantees is parent-first.
///
/// Written as a rule rather than as a table so that a mount point added to
/// `stage0_plan` by somebody else is classified without this file being
/// touched — and so that one landing somewhere genuinely new, like `/var/lib`,
/// comes out as [`Provision::InImage`] and fails
/// `every_stage0_mount_point_that_lands_on_this_filesystem_is_in_the_tree`
/// instead of failing on a card.
pub fn provision(target: &str) -> Provision {
    let plan = stage0_plan();
    let own_position = plan.iter().position(|s| s.target == target);

    let earlier_in_plan = plan.iter().enumerate().filter_map(|(i, s)| {
        match own_position {
            // A target already in the plan is covered only by a mount that
            // happens before it.
            Some(p) if i >= p => None,
            _ => Some(s.target),
        }
    });

    // Deepest wins, so that a two-level nesting reports the mount that is
    // actually on top rather than the outermost one.
    let mut covered_by: Option<&'static str> = None;
    for candidate in std::iter::once(DEV_MOUNT_POINT).chain(earlier_in_plan) {
        let is_ancestor = Path::new(target)
            .ancestors()
            .skip(1)
            .any(|a| a == Path::new(candidate));
        if is_ancestor && covered_by.is_none_or(|c| candidate.len() > c.len()) {
            covered_by = Some(candidate);
        }
    }

    match covered_by {
        Some(covered_by) => Provision::UnderRuntimeMount { covered_by },
        None => Provision::InImage,
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // Test-only since the `/etc` ruling (ADR-007 §3.5): this crate no longer
    // renders core's definitions onto a card, but the tests still assert that
    // the definitions exist and that none of them reaches the image.
    use rivet_core::etc;

    /// Absolute, because [`tree`] refuses anything else — and invented here
    /// rather than taken from the filesystem, since this module performs no
    /// I/O and a test that needed a real binary would not run on the build
    /// machine before one had been cross-compiled.
    /// **`PLAN.md` §72.3, as a test.** The binary was added to
    /// `tools/card/build.sh`'s `$OURS`, was built and stripped, and **was not
    /// on the card** — because a name in that list reaches the build and not
    /// the image. Nothing anywhere asserted that a binary handed to this crate
    /// arrives at a path, so the omission was invisible until a board was
    /// asked what was in `/system/bin`.
    #[test]
    fn an_mcpd_that_was_given_reaches_the_path_the_control_plane_is_installed_at() {
        let mut given = inputs();
        given.mcpd = PathBuf::from("/build/rivet-mcpd");
        let placed = tree(&given).expect("a valid tree");
        assert!(
            placed.iter().any(|n| n.path == MCPD_PATH),
            "rivet-mcpd was given and does not reach {MCPD_PATH}"
        );
    }

    /// **Every card carries one**, which is what makes `rivet-mcpd`'s
    /// `enabled: false` mean one thing. The test that stood here asserted the
    /// opposite — that a card built without an `mcpd` installs nothing at that
    /// path — and it described a card this build can no longer produce.
    #[test]
    fn no_card_can_be_built_without_the_control_plane() {
        let placed = tree(&inputs()).expect("a valid tree");
        assert!(
            placed.iter().any(|n| n.path == MCPD_PATH),
            "the default inputs produced a card with no control plane"
        );
    }

    // -----------------------------------------------------------------
    // Wireless - ADR-004, installed 2026-08-30
    // -----------------------------------------------------------------

    /// **A card with no wireless is a supported machine, not a broken one.**
    /// `ADR-004` §2 brings up wired first and §2.4 keeps the wired interface,
    /// and every Rivet board so far has run on a static address.
    #[test]
    fn a_build_given_no_wireless_installs_none_of_it() {
        let placed = tree(&inputs()).expect("a valid tree");
        for absent in [SUPPLICANT_PATH, WPA_CLI_PATH, UDHCPC_PATH, FIRMWARE_BIN] {
            assert!(
                !placed.iter().any(|n| n.path == absent),
                "{absent} was installed by a build that was not given it"
            );
        }
    }

    /// The three programs land in `/system/bin` like everything else - there is
    /// no `usr/sbin` on a Rivet machine (`ADR-008` §3).
    #[test]
    fn the_wireless_programs_land_in_system_bin() {
        let mut i = inputs();
        i.supplicant = Some(PathBuf::from("/br/target/usr/sbin/wpa_supplicant"));
        i.wpa_cli = Some(PathBuf::from("/br/target/usr/sbin/wpa_cli"));
        i.udhcpc = Some(PathBuf::from("/br/target/bin/busybox"));
        i.dhcp_hook = Some(PathBuf::from("/build/rivet-dhcp-hook"));
        let placed = tree(&i).expect("a valid tree");
        for path in [SUPPLICANT_PATH, WPA_CLI_PATH, UDHCPC_PATH] {
            let node = placed
                .iter()
                .find(|n| n.path == path)
                .unwrap_or_else(|| panic!("{path} was not installed"));
            assert!(path.starts_with("/system/bin/"), "{path}");
            assert_eq!(node.mode, EXEC_MODE, "{path} is not executable");
        }
    }

    /// **`udhcpc` is installed under its own name, from a file called
    /// `busybox`.** The binary carries exactly one applet, and with one applet
    /// compiled in the multiplexer dispatches to it whatever `argv[0]` says - so
    /// a file named `busybox` would advertise three hundred programs that are
    /// not in it.
    #[test]
    fn the_dhcp_client_is_named_for_what_it_is_rather_than_where_it_came_from() {
        let mut i = inputs();
        i.udhcpc = Some(PathBuf::from("/br/target/bin/busybox"));
        i.dhcp_hook = Some(PathBuf::from("/build/rivet-dhcp-hook"));
        let placed = tree(&i).expect("a valid tree");
        assert!(placed.iter().any(|n| n.path == UDHCPC_PATH));
        assert!(
            !placed.iter().any(|n| n.path.ends_with("/busybox")),
            "a file called busybox reached the card"
        );
    }

    /// **The silent failure, refused at build time.** udhcpc would run,
    /// broadcast, take an offer, and exec a path that is not on the card -- and
    /// the machine would look like one whose network is simply down.
    #[test]
    fn a_dhcp_client_with_nothing_to_exec_is_refused() {
        let mut i = inputs();
        i.udhcpc = Some(PathBuf::from("/br/target/bin/busybox"));
        assert_eq!(tree(&i), Err(SystemError::DhcpClientWithoutItsHook));
    }

    /// The reverse is NOT refused: a hook with no client is inert, and is what
    /// every card built before the client existed looks like.
    #[test]
    fn a_hook_without_a_client_is_allowed_because_it_is_inert() {
        let mut i = inputs();
        i.dhcp_hook = Some(PathBuf::from("/build/rivet-dhcp-hook"));
        let placed = tree(&i).expect("a hook alone is a valid card");
        assert!(placed.iter().any(|n| n.path == DHCP_HOOK_PATH));
    }

    /// **The firmware names were MEASURED**, `ADR-004` §8.1, off a real board:
    /// the driver appends the first device-tree compatible string to the base
    /// name. A test that recomputed them would be asserting my arithmetic
    /// rather than the board's answer.
    #[test]
    fn the_firmware_lands_under_the_names_the_driver_asks_for() {
        let mut i = inputs();
        i.wifi_firmware = Some(PathBuf::from("/br/target/lib/firmware/brcm"));
        let placed = tree(&i).expect("a valid tree");
        for path in [FIRMWARE_BIN, FIRMWARE_CLM, FIRMWARE_NVRAM] {
            assert!(
                placed.iter().any(|n| n.path == path),
                "{path} is missing - ADR-004 §8 warns a missing clm_blob does not FAIL, it makes \
                 association misbehave"
            );
        }
        assert!(
            placed.iter().any(|n| n.path == FIRMWARE_BRCM_DIR && n.is_directory()),
            "the directory the kernel's loader searches was not created"
        );
    }

    /// Firmware is DATA. A file the kernel reads should not be marked
    /// executable, because that invites the question of what running it does.
    #[test]
    fn the_firmware_is_not_executable() {
        let mut i = inputs();
        i.wifi_firmware = Some(PathBuf::from("/br/target/lib/firmware/brcm"));
        let placed = tree(&i).expect("a valid tree");
        let bin = placed
            .iter()
            .find(|n| n.path == FIRMWARE_BIN)
            .expect("installed");
        assert_eq!(bin.mode, 0o444, "firmware is executable");
        assert_ne!(bin.mode, EXEC_MODE);
    }

    /// **Each source basename is spelled once**, `ADR-011`. The pairing lives in
    /// `FIRMWARE_FILES`; this asserts the destinations really are composed from
    /// it rather than written out a second time.
    #[test]
    fn every_firmware_file_comes_from_the_one_list() {
        let dir = PathBuf::from("/somewhere/brcm");
        let mut i = inputs();
        i.wifi_firmware = Some(dir.clone());
        let placed = tree(&i).expect("a valid tree");
        for (name, destination) in FIRMWARE_FILES {
            let node = placed
                .iter()
                .find(|n| n.path == *destination)
                .unwrap_or_else(|| panic!("{destination} missing"));
            assert_eq!(
                node.content,
                Content::CopiedFrom(dir.join(name)),
                "{destination} is not read from {name}"
            );
        }
    }

    /// **A card given the proxy carries it, executable, at the path the
    /// supervisor executes** — `ADR-060`.
    #[test]
    fn a_card_given_the_proxy_carries_it_at_the_path_the_supervisor_runs() {
        let binary = PathBuf::from("/build/aarch64-unknown-linux-musl/release/rivet-proxyd");
        let mut i = inputs();
        i.proxyd = Some(binary.clone());
        let placed = tree(&i).expect("a valid tree");
        let node = placed
            .iter()
            .find(|n| n.path == PROXYD_PATH)
            .expect("the proxy is on the card");
        assert_eq!(node.content, Content::CopiedFrom(binary));
        assert_eq!(node.mode, 0o755, "the supervisor execs it");
        assert_eq!(PROXYD_PATH, "/system/bin/rivet-proxyd");
    }

    /// And a card not given it has no node at all, rather than an empty one —
    /// the supervisor then refuses the spawn once, loudly, on a read-only slot.
    #[test]
    fn a_card_not_given_the_proxy_has_no_node_for_it() {
        let placed = tree(&inputs()).expect("a valid tree");
        assert!(!placed.iter().any(|n| n.path == PROXYD_PATH));
    }

    /// **A card given the name service carries it, executable, at the path the
    /// supervisor executes** — `ADR-047`. And one not given it has no node.
    #[test]
    fn a_card_given_the_name_service_carries_it_at_the_path_the_supervisor_runs() {
        let binary = PathBuf::from("/build/aarch64-unknown-linux-musl/release/rivet-dnsd");
        let mut i = inputs();
        i.dnsd = Some(binary.clone());
        let placed = tree(&i).expect("a valid tree");
        let node = placed
            .iter()
            .find(|n| n.path == DNSD_PATH)
            .expect("the name service is on the card");
        assert_eq!(node.content, Content::CopiedFrom(binary));
        assert_eq!(node.mode, 0o755, "the supervisor execs it");
        assert_eq!(DNSD_PATH, "/system/bin/rivet-dnsd");
        let without = tree(&inputs()).expect("a valid tree");
        assert!(!without.iter().any(|n| n.path == DNSD_PATH));
    }

    fn inputs() -> Inputs {
        let build = "/build/aarch64-unknown-linux-musl/release";
        Inputs {
            init: PathBuf::from(format!("{build}/rivet-initd")),
            supervisor: PathBuf::from(format!("{build}/rivet-sysd")),
            cli: PathBuf::from(format!("{build}/rivet")),
            netd: PathBuf::from(format!("{build}/rivet-netd")),
            // Not from the same directory as the four above, and deliberately
            // so: `ADR-009` §6 has Buildroot build this one, so it comes out of
            // a different tree entirely and a fixture that pretended otherwise
            // would hide that.
            timed: PathBuf::from(format!("{build}/rivet-timed")),
            mdnsd: PathBuf::from(format!("{build}/rivet-mdnsd")),
            // Buildroot's, from a different tree entirely, for `ADR-009` §6's
            // reason exactly: a C mechanism is not built here.
            git: PathBuf::from("/buildroot/target/usr/bin/git"),
            http_backend: PathBuf::from(
                "/buildroot/target/usr/libexec/git-core/git-http-backend",
            ),
            deploy: PathBuf::from(format!("{build}/rivet-deploy")),
            // Absent for the same reason `nushell` is below: the default
            // fixture is the *minimal supported* card, so a test that wants
            // either optional binary has to say so and is visible doing it.
            femto: None,
            splash: None,
            bluetoothd: None,
            launcher: None,
            displayd: None,
            proxyd: None,
            dnsd: None,
            supplicant: None,
            wpa_cli: None,
            udhcpc: None,
            dhcp_hook: None,
            wifi_firmware: None,
            // Absent by default, on `nushell`'s terms: an image with no loader is a
            // supported image (every Rivet binary is static), and it is what every
            // card built before 2026-08-31 was.
            loader: None,
            libraries: Vec::new(),
            libraries_at: Vec::new(),
            ca_bundle: None,
            system_font: None,
            mono_font: None,
            icon_font: None,
            // The default fixture is the MINIMAL supported card, so it carries
            // no zone database and can only keep UTC -- which is every card
            // built before 2026-09-02.
            zoneinfo: Vec::new(),
            // Absent by default, exactly as femto is: the fixture is a card that
            // does not carry the optional binaries, which is a supported card.
            mcpd: PathBuf::from(format!("{build}/rivet-mcpd")),
            // The card the ruling that makes `nu` optional promises keeps working: no `nu`,
            // and a login shell anyway. It is the *default* fixture rather than
            // a special case because it is the one every card built before
            // 2026-08-24 was, and because a fixture that always shipped `nu`
            // could not notice that the without-`nu` path had broken.
            nushell: None,
            // Absent for the same reason `nushell` is, and it is a different
            // reason: `ADR-005` §7.4 has not been ruled, so an image WITHOUT a
            // debugger is the state of every card and the one this fixture must
            // represent. If §7.4 ever rules them in unconditionally, this is
            // the line that changes and the tests below say which way.
            gdbserver: None,
            strace: None,
            // One trusted key, because an image with none is refused
            // (`ADR-028` §5.5.5) and a fixture that leant on `--no-trust`
            // would test the escape hatch rather than the ordinary card.
            trust: TrustStore::new(vec![rivet_core::package::trust_store::Entry {
                key: rivet_core::package::port::PublicKey([0x11; 32]),
                label: "rivet-system".to_string(),
                may_sign: rivet_core::package::port::MaySign::these(&[
                    rivet_core::package::Kind::update(),
                ]),
            }]),
            arch: "aarch64".to_string(),
            development_only: false,
            login_shells: LOGIN_SHELLS,
        }
    }

    /// The same image, with `nu` in it — `ADR-005` §5.1's login shell.
    fn inputs_with_nushell() -> Inputs {
        Inputs {
            nushell: Some(PathBuf::from(
                "/build/aarch64-unknown-linux-musl/release/nu",
            )),
            ..inputs()
        }
    }

    /// The same image, with the debugging mechanisms in it — `ADR-005` §7.
    ///
    /// Their sources are under a Buildroot tree rather than beside the Rust
    /// binaries, for the reason `inputs`'s `git` field carries: these are C
    /// mechanisms and `ADR-009` §6 builds them somewhere else entirely.
    fn inputs_with_debuggers() -> Inputs {
        Inputs {
            gdbserver: Some(PathBuf::from("/buildroot/target/usr/bin/gdbserver")),
            strace: Some(PathBuf::from("/buildroot/target/usr/bin/strace")),
            ..inputs()
        }
    }

    /// The same image, told that some other path is its only candidate shell.
    ///
    /// **Rewritten 2026-08-23.** This used to pass `INIT_PATH`, because the
    /// `rivet` CLI did not exist and something real had to stand in for it so
    /// that the *accepting* half of [`check_login_shells`] meant anything. The
    /// CLI exists, [`LOGIN_SHELLS`] names it, and `main` now drives the
    /// accepting arm on every build — so what this helper is for has inverted:
    /// it supplies the values that must be **refused**.
    ///
    /// **Rewritten again 2026-08-24**, when the guard became "at least one of".
    /// A single-element list is now what drives the refusing arm, because a
    /// list containing `CLI_PATH` is accepted whatever else is in it.
    fn inputs_with_login_shells(paths: &'static [&'static str]) -> Inputs {
        Inputs {
            login_shells: paths,
            ..inputs()
        }
    }

    /// A path that is not in the image and is not going to be.
    ///
    /// **Was `/system/bin/nu` until 2026-08-24**, on the argument that naming
    /// the shell `ADR-005` §5.1 rules in made the refusal about a real future
    /// mistake. That day arrived: `nu` *is* installable now, so using it here
    /// would leave the refusing arm asserting something the build can satisfy.
    /// The stand-in is a shell Rivet has ruled out entirely (`ADR-005` §5.4),
    /// which is the property the fixture actually needs.
    const NOT_IN_THE_IMAGE: &str = "/system/bin/bash";

    fn built() -> Vec<Node> {
        // Neither `unwrap` nor `expect`: both are warned across the workspace
        // and a test is not exempt. The assertion is what reports a fixture
        // that stopped being accepted; the default is only so this returns.
        let t = tree(&inputs());
        assert!(t.is_ok(), "the fixture inputs should be accepted: {t:?}");
        t.unwrap_or_default()
    }

    // -- the contract with rivet-core ---------------------------------------

    /// **The cross-module test.** `stage0_plan` and this tree are written by
    /// different people in different crates, and the failure they produce
    /// together is a `Degraded` boot with a message about a read-only
    /// filesystem — correct, and about the card rather than about the code.
    ///
    /// Deliberately expressed against [`provision`] rather than against a list
    /// of three paths: a mount point added to the plan tomorrow is checked by
    /// this test on the day it is added.
    #[test]
    fn every_stage0_mount_point_that_lands_on_this_filesystem_is_in_the_tree() {
        let t = built();
        for spec in stage0_plan() {
            if provision(spec.target) == Provision::InImage {
                let node = find(&t, spec.target);
                assert!(
                    node.is_some(),
                    "{} is mounted by PID 1 and the mkdir for it lands on read-only erofs, \
                     so it must be in the image",
                    spec.target
                );
                assert!(
                    node.is_some_and(Node::is_directory),
                    "{} must be a directory",
                    spec.target
                );
            }
        }
    }

    /// The other half, and the one that stops the first being satisfied by
    /// shipping everything. A directory under a filesystem that is mounted at
    /// runtime is **hidden** on the running machine — visible to whoever built
    /// the image and to nobody else.
    #[test]
    fn a_stage0_target_under_a_runtime_mount_is_left_out_because_it_would_be_hidden() {
        let t = built();
        for spec in stage0_plan() {
            if let Provision::UnderRuntimeMount { covered_by } = provision(spec.target) {
                assert!(
                    find(&t, spec.target).is_none(),
                    "{} is inside {covered_by}, which is mounted over it at runtime; \
                     a directory here would be invisible on the booted machine",
                    spec.target
                );
            }
        }
    }

    /// The classification itself, whole-list, so that changing one is a
    /// decision somebody takes on purpose rather than a side effect.
    #[test]
    fn the_eight_stage0_targets_are_classified_five_and_three() {
        let got: Vec<_> = stage0_plan()
            .iter()
            .map(|s| (s.target, provision(s.target)))
            .collect();
        assert_eq!(
            got,
            [
                ("/proc", Provision::InImage),
                ("/dev", Provision::InImage),
                ("/sys", Provision::InImage),
                (
                    "/sys/fs/cgroup",
                    Provision::UnderRuntimeMount { covered_by: "/sys" }
                ),
                (
                    "/dev/pts",
                    Provision::UnderRuntimeMount { covered_by: "/dev" }
                ),
                (
                    "/dev/shm",
                    Provision::UnderRuntimeMount { covered_by: "/dev" }
                ),
                ("/run", Provision::InImage),
                ("/tmp", Provision::InImage),
            ]
        );
    }

    /// `/dev` needs a directory in the image whether or not anything in the
    /// plan mounts it — and **the reason changed on 2026-08-24 while the
    /// requirement did not.**
    ///
    /// This test used to assert `/dev` was absent from `stage0_plan`, on the
    /// grounds that the kernel mounts devtmpfs before PID 1 (`ADR-024` §3).
    /// That holds on a board and not under the emulator, whose root is an
    /// initramfs the automount skips, so PID 1 now mounts it itself. The
    /// directory is still the image's to provide: a mount with no mount point
    /// fails, and `/system` is read-only by then.
    ///
    /// It must appear **exactly once**. It was hand-written into the node list
    /// *and* produced by the plan-driven pass, and the tree carried it twice.
    #[test]
    fn dev_is_in_the_image_exactly_once_now_that_the_plan_also_mounts_it() {
        assert!(stage0_plan().iter().any(|s| s.target == DEV_MOUNT_POINT));
        assert_eq!(provision(DEV_MOUNT_POINT), Provision::InImage);
        let tree = built();
        assert!(find(&tree, DEV_MOUNT_POINT).is_some_and(Node::is_directory));
        assert_eq!(
            tree.iter().filter(|n| n.path == DEV_MOUNT_POINT).count(),
            1,
            "/dev is in the image twice — the hand-written node and the plan-driven pass both added it (ADR-011: one authority, and it is the plan)"
        );
    }

    /// The rule generalises, and this is the case that proves it does. A mount
    /// point somewhere genuinely new must come out as [`Provision::InImage`],
    /// which is what makes the drift test above fail loudly rather than
    /// silently passing.
    #[test]
    fn a_target_with_no_mounted_ancestor_is_the_images_problem() {
        assert_eq!(provision("/var/lib/something"), Provision::InImage);
        assert_eq!(provision("/state/config"), Provision::InImage);
    }

    /// Deepest wins. `/run` is in the plan, so anything under it is the
    /// tmpfs's, not the image's — and it must report `/run` rather than `/`.
    #[test]
    fn a_path_under_a_planned_mount_is_reported_against_the_deepest_one() {
        assert_eq!(
            provision("/run/rivet/notify"),
            Provision::UnderRuntimeMount { covered_by: "/run" }
        );
    }

    /// A prefix is not an ancestor. `/system` starts with `/sys`, and treating
    /// that as containment would leave `/system` out of the image and the board
    /// with no PID 1 at all.
    #[test]
    fn a_string_prefix_is_not_a_parent_directory() {
        assert_eq!(provision("/system"), Provision::InImage);
        assert_eq!(provision("/system/bin"), Provision::InImage);
        assert!(find(&built(), "/system/bin").is_some());
    }

    // -- the tree itself -------------------------------------------------------

    /// Whole-list, on the same reasoning as `rivet-core`'s
    /// `stage0_plan_is_these_seven_in_this_order`: adding an entry to a signed
    /// artifact should require changing a test that says what the artifact is.
    ///
    /// **The count has moved in both directions, and that is the point.** The
    /// five `/etc` files arrived when this crate began generating them and left
    /// when the maintainer ruled that the runtime projection owns the whole of
    /// `/etc` (`ADR-007` §3.5); binaries have been added one at a time and one
    /// has been removed. This test is what made each of those visible as a
    /// change to what the card *is*, which is the entire reason it lists paths
    /// instead of counting them.
    ///
    /// **The name carries no number**, deliberately. `AGENTS.md` §7 is about a
    /// count in a name going stale, and this one's did — twice — while the
    /// test went on passing.
    ///
    /// Two entries are worth a sentence each because they are the exceptions:
    ///
    /// * `git-http-backend` is a SECOND file from a package already present
    ///   rather than a new one. `PLAN.md` §84.1 measured the inodes to prove
    ///   that, after `git http-backend` answered *"not a git command"* on a
    ///   running board.
    /// * `rivet-deploy` is the one binary here that **nothing on the card
    ///   starts** — the maintainer's 2026-08-24 install-request ruling's open half
    ///   rather than an omission in this crate.
    #[test]
    fn the_stage0_image_is_exactly_these_paths() {
        let nodes = built();
        let paths: Vec<&str> = nodes.iter().map(|n| n.path.as_ref()).collect();
        assert_eq!(
            paths,
            [
                "/",
                "/dev",
                "/etc",
                "/proc",
                "/run",
                "/state",
                "/sys",
                "/system",
                "/system/arch",
                "/system/bin",
                "/system/bin/git",
                "/system/bin/git-http-backend",
                "/system/bin/rivet",
                "/system/bin/rivet-deploy",
                "/system/bin/rivet-initd",
                "/system/bin/rivet-mcpd",
                "/system/bin/rivet-mdnsd",
                "/system/bin/rivet-netd",
                "/system/bin/rivet-sysd",
                "/system/bin/rivet-timed",
                "/system/ext",
                "/system/trust",
                "/tmp",
            ]
        );
    }

    #[test]
    fn the_six_binaries_are_copied_from_the_inputs_and_are_executable() {
        let t = built();
        let i = inputs();
        for (path, source) in [
            (INIT_PATH, i.init),
            (SUPERVISOR_PATH, i.supervisor),
            (CLI_PATH, i.cli),
            (NETD_PATH, i.netd),
            (TIMED_PATH, i.timed),
            (DEPLOY_PATH, i.deploy),
        ] {
            let node = find(&t, path);
            assert!(node.is_some(), "{path} is missing from the image");
            assert!(
                node.is_some_and(|n| n.content == Content::CopiedFrom(source.clone())),
                "{path} does not come from the input the caller supplied"
            );
            // The literal, not `EXEC_MODE`. Asserting a constant against
            // itself is a tautology, and this test proved it: injecting
            // `EXEC_MODE = 0o644` was **not caught** until this line stopped
            // comparing the value with its own definition.
            assert!(
                node.is_some_and(|n| n.mode == 0o755),
                "{path} is not mode 0755"
            );
            assert!(
                node.is_some_and(|n| n.mode & 0o111 == 0o111),
                "{path} is not executable; the kernel cannot exec PID 1"
            );
        }
    }

    /// PID 1 execs this literal. The two copies live in different crates and
    /// nothing links them, so this is the tripwire named on
    /// [`SUPERVISOR_PATH`]'s documentation.
    #[test]
    fn the_supervisor_path_matches_the_one_pid_1_execs() {
        // `cmd/rivet-initd/src/supervise.rs`, `pub const SUPERVISOR_PATH`.
        assert_eq!(SUPERVISOR_PATH, "/system/bin/rivet-sysd");
        // And `cmdline.txt`'s `init=`, which `boot` writes.
        assert_eq!(INIT_PATH, "/system/bin/rivet-initd");
    }

    /// The same tripwire for the CLI, and this one has teeth in an unusual
    /// direction: `cmd/rivet` compares a `-c` command line's first word against
    /// its own `INSTALL_PATH`, so if these two drift, a non-interactive
    /// invocation of `/system/bin/rivet --version` is refused by the binary
    /// that *is* `/system/bin/rivet`.
    #[test]
    fn the_cli_path_matches_the_one_the_cli_answers_to() {
        // `cmd/rivet/src/invocation.rs`, `pub const INSTALL_PATH`.
        assert_eq!(CLI_PATH, "/system/bin/rivet");
        // And one of the values `rivet_core::etc` may write into /etc/shells
        // and root's shell field, which `getusershell` reads before any login
        // is admitted.
        assert!(LOGIN_SHELLS.contains(&CLI_PATH));
    }

    /// `rivet-netd` is installed and **nothing in this image starts it**,
    /// which belongs to `cmd/rivet-sysd`.
    /// Asserted here so the half that is done is not mistaken for the whole:
    /// this crate can put a binary on a card and cannot make anything run it.
    #[test]
    fn netd_is_on_the_card_even_though_nothing_in_this_image_starts_it() {
        let t = built();
        let node = find(&t, NETD_PATH);
        assert!(node.is_some(), "{NETD_PATH} is not in the image");
        assert!(node.is_some_and(|n| n.mode & 0o111 != 0));
        // PID 1 execs exactly one child and it is the supervisor, so nothing on
        // this card reaches netd. ADR-017 §3.1.
        assert_ne!(NETD_PATH, SUPERVISOR_PATH);
    }

    #[test]
    fn everything_is_owned_by_root_because_the_builder_is_not() {
        for node in built() {
            assert_eq!(node.uid, 0, "{} is not root-owned", node.path);
            assert_eq!(node.gid, 0, "{} is not root-owned", node.path);
        }
        // And the tree alone cannot achieve that — `mkfs.erofs` copies the
        // staging tree's ownership unless it is told otherwise.
        assert!(MKFS_EROFS_OWNERSHIP_ARGS.contains(&"--all-root"));
    }

    #[test]
    fn every_directory_is_traversable_by_everyone_and_writable_by_nobody() {
        let t = built();
        for node in t.iter().filter(|n| n.is_directory()) {
            assert_eq!(node.mode, 0o755, "{} has the wrong mode", node.path);
            assert_eq!(
                node.mode & 0o222,
                0o200,
                "{} is group- or world-writable",
                node.path
            );
        }
    }

    /// Nothing in the base image is setuid or setgid. `ADR-017` §3.1 already
    /// runs the supervisor as root, so there is nothing for a setuid bit to buy
    /// and it is the sort of thing that arrives without a reason.
    #[test]
    fn no_entry_carries_a_setuid_or_setgid_bit() {
        for node in built() {
            assert_eq!(node.mode & 0o6000, 0, "{} is setuid or setgid", node.path);
        }
    }

    #[test]
    fn every_entry_says_why_it_is_there() {
        for node in built() {
            assert!(
                !node.reason.is_empty(),
                "{} has no stated reason",
                node.path
            );
        }
    }

    // -- /etc ------------------------------------------------------------------

    /// `/etc` is a directory, and it is **empty by ruling**.
    ///
    /// **The maintainer ruled 2026-08-23** that `/etc` stays empty on the image
    /// until the runtime projection renders all of it — one generator rather
    /// than two (`ADR-007` §3.5, `ADR-011`). This replaces a test that asserted
    /// the opposite earlier the same day.
    ///
    /// It is an assertion rather than a deletion because *empty* is now a
    /// decision, and a decision that nothing tests is one a future change
    /// reverses without ever learning it was made.
    #[test]
    fn etc_is_empty_because_one_generator_owns_it_and_it_is_not_this_one() {
        let t = built();
        assert!(find(&t, ETC).is_some_and(Node::is_directory));
        let under: Vec<_> = t
            .iter()
            .filter(|n| n.path.starts_with("/etc/"))
            .map(|n| n.path.as_ref())
            .collect();
        assert!(
            under.is_empty(),
            "the image is generating /etc again: {under:?} — see ADR-007 §3.5"
        );
    }

    /// The definitions survived the ruling; only the second renderer did not.
    ///
    /// **This inverts a test of the same name-shape from earlier on
    /// 2026-08-23**, which asserted that every file `rivet-core` renders
    /// reaches the image. The ruling makes that false on purpose. What must
    /// still be true is that the *knowledge* was not thrown away with the
    /// renderer: `rivet_core::etc` cost a sweep of the linked `libc.a` to
    /// establish, it found a ninth musl path `ADR-008` §5.1 had only predicted,
    /// and the runtime projection is what will render it.
    ///
    /// So this asserts core still defines them and this crate still ships none
    /// of them — the two halves of the ruling, in one place.
    #[test]
    fn core_still_defines_etc_even_though_the_image_no_longer_renders_it() {
        let defined = etc::image_etc(&etc::EtcInputs {
            login_shells: LOGIN_SHELLS,
            // A machine with nothing installed; this crate renders no /etc at all.
            ..etc::EtcInputs::default()
        });
        assert!(
            !defined.is_empty(),
            "rivet-core's /etc definitions were deleted along with the renderer; \
             the runtime projection has nothing left to render"
        );
        let t = built();
        for file in defined {
            assert!(
                find(&t, file.path).is_none(),
                "{} is on the card; the image must not generate /etc (ADR-007 §3.5)",
                file.path
            );
        }
    }

    /// And the other direction: nothing under `/etc` may be invented here.
    /// `ADR-007` §3.5 makes `/etc` generated output and never authored, and a
    /// file added in this module would be exactly the second authority
    /// `ADR-011` forbids — one this crate owns and the runtime generator has
    /// never heard of.
    #[test]
    fn nothing_under_etc_is_authored_in_this_module() {
        let defined: Vec<_> = etc::image_etc(&etc::EtcInputs {
            login_shells: LOGIN_SHELLS,
            // A machine with nothing installed; this crate renders no /etc at all.
            ..etc::EtcInputs::default()
        })
        .into_iter()
        .map(|f| f.path)
        .collect();
        for node in built().iter().filter(|n| n.path.starts_with("/etc/")) {
            assert!(
                defined.contains(&node.path.as_ref()),
                "{} is in the image but is not one of rivet-core's",
                node.path
            );
        }
    }

    // ---- the zone database, ADR-021 §9, 2026-09-02 ------------------------

    /// **The database lands where the projection looks for it**, which is the
    /// one thing that has to be true for a `timezone` key to do anything:
    /// `cmd/rivet-sysd` composes `ZONEINFO_DIR/<zone>` and reads it.
    #[test]
    fn the_zone_database_lands_where_the_projection_reads_it() {
        let mut inputs = inputs();
        inputs.zoneinfo = vec![
            (
                "America/Chicago".to_string(),
                PathBuf::from("/usr/share/zoneinfo/America/Chicago"),
            ),
            ("UTC".to_string(), PathBuf::from("/usr/share/zoneinfo/UTC")),
        ];
        let t = tree(&inputs).expect("a tree");
        let chicago = find(&t, "/system/share/zoneinfo/America/Chicago")
            .expect("the zone the projection would read");
        assert_eq!(
            chicago.content,
            Content::CopiedFrom(PathBuf::from("/usr/share/zoneinfo/America/Chicago"))
        );
        // Data, not a program — `Node::data`'s 0444. A tzfile is read by libc.
        assert_eq!(chicago.mode, 0o444);
        assert!(find(&t, "/system/share/zoneinfo").is_some_and(Node::is_directory));
        assert!(find(&t, "/system/share/zoneinfo/UTC").is_some());
    }

    /// **A card with no database is a supported card** — it keeps UTC, which is
    /// every card built before 2026-09-02. The directory is absent rather than
    /// present and empty, for `rivet_core::etc`'s reason: an empty
    /// `/system/share/zoneinfo` is a card that looks as though it could keep a
    /// timezone.
    #[test]
    fn a_card_with_no_zone_database_carries_no_zoneinfo_directory() {
        let t = built();
        assert!(
            !t.iter()
                .any(|n| n.path.starts_with("/system/share/zoneinfo")),
            "a minimal card carries a zone database"
        );
    }

    /// The three `ADR-008` §5.1 paths that are deliberately absent. Each is
    /// argued in `rivet_core::etc`; this asserts the argument reached the
    /// card.
    ///
    /// **`/etc/localtime` is still here on 2026-09-02, and its meaning
    /// changed.** It used to be absent because nothing rendered it; it is now
    /// absent because the *runtime* renders it onto a tmpfs that would mask
    /// anything shipped here. Same assertion, different reason, and worth
    /// stating so that a future reader does not "fix" it after reading
    /// `ADR-021` §9.
    #[test]
    fn the_runtime_owned_musl_paths_are_not_on_the_card() {
        let t = built();
        for path in ["/etc/resolv.conf", "/etc/localtime", "/etc/services"] {
            assert!(
                find(&t, path).is_none(),
                "{path} is in the image; a stale copy on a read-only filesystem cannot be \
                 corrected on the running machine"
            );
        }
    }

    /// **This replaces `stage_0_generates_no_etc_and_that_is_recorded_here_not_papered_over`,
    /// which recorded the gap this change closes.**
    ///
    /// The old test's real content was the observation that nothing mounts
    /// anything at `/etc`. That is still true and it is now load-bearing in the
    /// other direction: because nothing is mounted over `/etc`, these files are
    /// the ones musl actually reads on a booted board rather than being hidden
    /// by a tmpfs. The day something *is* mounted there, the generator behind it
    /// must render `rivet_core::etc::image_etc` too — and this test failing is
    /// the prompt to check that it does.
    #[test]
    fn nothing_is_mounted_over_etc_so_these_files_are_the_live_ones() {
        assert!(
            !stage0_plan().iter().any(|s| s.target.starts_with(ETC)),
            "something now mounts over /etc: the generator behind it must render \
             rivet_core::etc::image_etc, or these files become invisible to musl"
        );
    }

    // -- the login shell -------------------------------------------------------

    /// This asserted `LOGIN_SHELL == None` until 2026-08-23 and was correct
    /// while `cmd/rivet` did not exist. It does, so the assertion inverted —
    /// and the *second* half of it is unchanged, because that half is what the
    /// constant is for: whatever [`LOGIN_SHELLS`] says must survive
    /// [`check_login_shells`] against this image's own tree.
    #[test]
    fn the_login_shell_this_image_names_is_a_binary_this_image_contains() {
        assert!(LOGIN_SHELLS.contains(&CLI_PATH));
        let t = built();
        assert!(check_login_shells(&t, LOGIN_SHELLS).is_ok());
        let node = find(&t, CLI_PATH);
        assert!(node.is_some_and(|n| !n.is_directory() && n.mode & 0o111 != 0));
    }

    /// **What a card carries, and what that is not.**
    ///
    /// A login needs a login shell binary **and** an `/etc/shells`
    /// on the running machine naming it, before any credential is looked at.
    /// The card carries the first; the second is the runtime projection's, and
    /// this asserts the image does not try to supply it.
    ///
    /// **The honest residue**, and it is the reason this test says "on the
    /// card" rather than "works": both facts are true of a built card and
    /// **nothing has ever run**. No login has been attempted on a Rivet board
    /// (`AGENTS.md` §6, `ADR-026` §4's right-hand column).
    #[test]
    fn the_facts_a_login_needs_are_on_the_card_and_none_of_them_has_run() {
        let t = built();
        // 1. The login shell binary.
        assert!(find(&t, CLI_PATH).is_some_and(|n| n.mode & 0o111 != 0));
        // 2. No /etc/shells *in the image*, and that is correct rather than a
        //    gap: ADR-007 §3.5 gives the whole of /etc to the runtime
        //    projection, and a file here would be a second answer to what this
        //    machine's login shells are.
        assert!(
            find(&t, "/etc/shells").is_none(),
            "the image is generating /etc again — this test's premise changed"
        );
        // And nothing here starts anything. PID 1 execs one child.
        assert_ne!(CLI_PATH, SUPERVISOR_PATH);
        assert_ne!(CLI_PATH, INIT_PATH);
    }

    #[test]
    fn a_login_shell_that_is_in_the_image_and_executable_is_accepted() {
        let t = built();
        assert_eq!(check_login_shells(&t, &[CLI_PATH]), Ok(()));
        assert_eq!(check_login_shells(&t, &[INIT_PATH]), Ok(()));
    }

    /// **Through `tree` itself**, which is the only thing that matters: the
    /// guard is worth nothing if `tree` stops calling it.
    ///
    /// Removing the call was injected and **was not caught** while the value
    /// came from a constant that was `None` — the call was a no-op and no test
    /// could reach it. That is why [`Inputs`] carries the field.
    ///
    /// **The subject changed 2026-08-23** and the change is what keeps this test
    /// able to fail: it used to name `/system/bin/rivet`, which is now *in*
    /// the image, so the same assertion would pass for the wrong reason. It
    /// named `nushell` instead — and **on 2026-08-24 that stopped working for
    /// the identical reason**, because `--nushell` puts `nu` in the image. The
    /// subject is now a shell `ADR-005` §5.4 rules out entirely, which is the
    /// only kind of path that cannot become installable behind this test's
    /// back.
    #[test]
    fn tree_refuses_to_build_an_image_whose_login_shell_it_does_not_contain() {
        assert!(
            find(&built(), NOT_IN_THE_IMAGE).is_none(),
            "this image now contains {NOT_IN_THE_IMAGE}; the refusal test needs a new subject"
        );
        assert_eq!(
            tree(&inputs_with_login_shells(&[NOT_IN_THE_IMAGE])),
            Err(SystemError::NoLoginShellInImage {
                candidates: vec![NOT_IN_THE_IMAGE.to_string()],
            })
        );
    }

    /// And the accepting half, so the refusal is not simply "always refuse":
    /// a shell that *is* in the image builds.
    ///
    /// **Narrowed 2026-08-23 by the `/etc` ruling**, then widened again later
    /// the same day. It used to assert the shell landed in a generated
    /// `/etc/shells`; the image no longer generates one (`ADR-007` §3.5), so the
    /// second half moved to where the file *is* made — `rivet-core`'s
    /// renderer, which the runtime projection will call.
    ///
    /// **What changed with the CLI**: this arm is no longer reachable only
    /// through the [`Inputs`] field. It is what [`LOGIN_SHELLS`] itself does, so
    /// it is driven twice — once through the constant, which is what `main`
    /// passes, and once through the field, which is what proves the parameter is
    /// honoured rather than ignored.
    ///
    /// **The `/etc` half is asserted against a card without `nu`**, because
    /// that is what `inputs()` builds and because the projection's inputs are a
    /// *probe result*, not this constant: the image contributes candidates, and
    /// which of them exist is the running machine's answer.
    #[test]
    fn tree_accepts_a_login_shell_it_contains_and_the_value_reaches_cores_renderer() {
        // Through the constant: exactly what `main` builds.
        let t = tree(&inputs());
        assert!(t.is_ok(), "the shipped configuration must build: {t:?}");
        // And through the field, with a different valid value, so that a `tree`
        // which ignored the parameter and read the constant would still fail
        // `tree_refuses_to_build_an_image_whose_login_shell_it_does_not_contain`.
        let t = tree(&inputs_with_login_shells(&[INIT_PATH]));
        assert!(t.is_ok(), "a shell that is in the image must build: {t:?}");

        let rendered = etc::image_etc(&etc::EtcInputs {
            login_shells: &[CLI_PATH],
            // A machine with nothing installed; this crate renders no /etc at all.
            ..etc::EtcInputs::default()
        });
        let shells = rendered.iter().find(|f| f.path == "/etc/shells");
        assert!(shells.is_some(), "core renders no /etc/shells at all");
        assert!(
            shells.is_some_and(|f| f.text().lines().any(|l| l == CLI_PATH)),
            "the login shell did not reach core's /etc/shells rendering"
        );
        // And the same path becomes root's shell field, so the account
        // `getpwnam` resolves and the `/etc/shells` `getusershell` reads agree.
        // Asserted against core's rendering rather than against the card, for
        // the reason above.
        let passwd = rendered.iter().find(|f| f.path == "/etc/passwd");
        assert!(
            passwd.is_some_and(|f| f.text().contains(&format!(":{CLI_PATH}
"))),
            "/etc/passwd and /etc/shells disagree about the login shell"
        );
    }

    /// The refusal, driven on both of the ways it can be wrong: a path that is
    /// not in the tree at all, and one that is there but is not a program.
    #[test]
    fn a_login_shell_the_image_does_not_contain_is_refused_rather_than_listed() {
        let t = built();
        assert_eq!(
            check_login_shells(&t, &[NOT_IN_THE_IMAGE]),
            Err(SystemError::NoLoginShellInImage {
                candidates: vec![NOT_IN_THE_IMAGE.to_string()],
            })
        );
        assert!(
            check_login_shells(&t, &["/system/bin"]).is_err(),
            "a directory is not a login shell"
        );
        assert!(
            check_login_shells(&t, &["/etc/shells"]).is_err(),
            "a file with no execute bit is not a login shell"
        );
        // A near-miss on the real path, which is the mistake a typo makes.
        assert!(
            check_login_shells(&t, &["/system/bin/rivet-cli"]).is_err(),
            "a path that merely starts with the CLI's was accepted"
        );
    }

    /// **The weakening, asserted rather than left to be inferred.**
    ///
    /// The ruling that makes `nu` optional requires a card built without `nu` to build and
    /// to work. The shipped `LOGIN_SHELLS` names `nu` first, so a guard that
    /// demanded every candidate would refuse the default build outright — and
    /// the failure would be a build error nobody could act on, since the ruling
    /// says that card is correct.
    ///
    /// Injecting `any` → `all` in [`check_login_shells`] is what this catches,
    /// and it is a one-word edit somebody will make while "tightening" the
    /// guard.
    #[test]
    fn a_missing_candidate_is_an_ordinary_card_and_not_a_build_failure() {
        let t = built();
        assert!(
            find(&t, NUSHELL_PATH).is_none(),
            "the default fixture now ships nu; this test no longer covers the without-nu card"
        );
        assert_eq!(check_login_shells(&t, LOGIN_SHELLS), Ok(()));
        assert!(
            tree(&inputs()).is_ok(),
            "the shipped LOGIN_SHELLS names nu, which the default build does not install"
        );
    }

    /// And the other direction: given `--nushell`, the binary lands at exactly
    /// the path the runtime projection probes for.
    ///
    /// **The two constants are the same string by construction** — both come
    /// from `rivet_core::etc` — so what this actually asserts is that `tree`
    /// installs it at [`NUSHELL_PATH`] rather than somewhere adjacent, and that
    /// it is executable. A `nu` that is on the card and not executable passes
    /// `getusershell()` and fails at `exec`, after authentication, naming
    /// nothing.
    #[test]
    fn given_a_nushell_the_image_puts_it_where_the_projection_looks() {
        let built = tree(&inputs_with_nushell());
        assert!(built.is_ok(), "an image with nu must build: {built:?}");
        let t = built.unwrap_or_default();
        let node = find(&t, NUSHELL_PATH);
        assert!(node.is_some(), "{NUSHELL_PATH} is not in the image");
        assert!(node.is_some_and(|n| !n.is_directory() && n.mode & 0o111 != 0));
        assert_eq!(NUSHELL_PATH, rivet_core::etc::NUSHELL_PATH);
        // The CLI is still there. The rule that keeps every present shell listed keeps it in /etc/shells
        // so that an account naming it is not refused before authentication,
        // which needs the binary to exist as much as the listing does.
        assert!(find(&t, CLI_PATH).is_some());
    }

    /// `femto` goes on the card, executable, at the path a person will type.
    ///
    /// An editor installed non-executable fails at `exec` after the operator has
    /// already decided to edit something — the same shape of late failure
    /// [`given_a_nushell_the_image_puts_it_where_the_projection_looks`] guards
    /// against for the login shell.
    #[test]
    fn given_a_femto_the_image_puts_it_on_the_card_executable() {
        let mut i = inputs();
        i.femto = Some(PathBuf::from(
            "/build/aarch64-unknown-linux-musl/release/femto",
        ));
        let built = tree(&i);
        assert!(built.is_ok(), "an image with femto must build: {built:?}");
        let t = built.unwrap_or_default();
        let node = find(&t, FEMTO_PATH);
        assert!(node.is_some(), "{FEMTO_PATH} is not in the image");
        assert!(node.is_some_and(|n| !n.is_directory() && n.mode & 0o111 != 0));
    }

    /// **A card without an editor is a supported card**, and the default
    /// fixture is that card. `FEMTO_PATH` states the rule; this holds it.
    #[test]
    fn without_a_femto_the_image_still_builds_and_carries_no_editor() {
        let built = tree(&inputs());
        assert!(built.is_ok(), "a card with no editor must build: {built:?}");
        assert!(
            find(&built.unwrap_or_default(), FEMTO_PATH).is_none(),
            "an editor appeared on a card that did not ask for one"
        );
    }

    /// A relative `--nushell` is the same defect as a relative `--cli`: the
    /// same command produces a different image from a different directory.
    /// Optional is about whether the argument was given.
    #[test]
    fn a_relative_nushell_path_is_refused_like_every_other_source() {
        let inputs = Inputs {
            nushell: Some(PathBuf::from("target/release/nu")),
            ..inputs()
        };
        assert_eq!(
            tree(&inputs),
            Err(SystemError::SourceNotAbsolute {
                role: NUSHELL_PATH,
                path: PathBuf::from("target/release/nu"),
            })
        );
    }

    // -- the debugging mechanisms, ADR-005 §7 ---------------------------------

    /// The default card has **no debugger**, and that is the state `ADR-005`
    /// §7.4 leaves undecided rather than an omission.
    ///
    /// Asserted in this direction because the failure it guards is the one that
    /// happens silently: somebody makes the argument required, or defaults it
    /// on, and every board acquires a debug stub without a ruling — which
    /// `ADR-028` would care about and which no build output would mention.
    #[test]
    fn without_the_arguments_no_debugger_reaches_the_card() {
        let t = built();
        assert!(
            find(&t, GDBSERVER_PATH).is_none(),
            "{GDBSERVER_PATH} is in an image that was not given one"
        );
        assert!(
            find(&t, STRACE_PATH).is_none(),
            "{STRACE_PATH} is in an image that was not given one"
        );
    }

    /// And the other direction, which is what `--gdbserver`/`--strace` are for.
    ///
    /// The execute bit is the load-bearing half. A `gdbserver` on the card
    /// without one fails at `exec` with `Permission denied`, which reads as a
    /// permissions or an account problem and is neither.
    #[test]
    fn given_the_binaries_they_land_executable_beside_the_others() {
        let built = tree(&inputs_with_debuggers());
        assert!(
            built.is_ok(),
            "an image with a debugger must build: {built:?}"
        );
        let t = built.unwrap_or_default();
        for path in [GDBSERVER_PATH, STRACE_PATH] {
            let node = find(&t, path);
            assert!(node.is_some(), "{path} is not in the image");
            assert!(
                node.is_some_and(|n| !n.is_directory() && n.mode & 0o111 != 0),
                "{path} is on the card and is not executable"
            );
        }
        // They are under /system/bin with the rest — ADR-008 §3, and NOT under
        // the /usr/bin Buildroot itself installs them to. ADR-009 §6 names that
        // convention as the boundary this image may not cross.
        assert_eq!(GDBSERVER_PATH, "/system/bin/gdbserver");
        assert_eq!(STRACE_PATH, "/system/bin/strace");
        // Everything else is still there. A debug card is the ordinary card
        // plus two files, not a variant with its own contents.
        for path in [
            INIT_PATH,
            SUPERVISOR_PATH,
            CLI_PATH,
            NETD_PATH,
            TIMED_PATH,
            DEPLOY_PATH,
        ] {
            assert!(
                find(&t, path).is_some(),
                "{path} vanished from a debug image"
            );
        }
    }

    /// A relative path is refused for the same reason every other source is,
    /// and asserted separately for each because they are pushed by separate
    /// `if let`s — a single test would pass with one of them missing.
    #[test]
    fn a_relative_debugger_path_is_refused_like_every_other_source() {
        assert_eq!(
            tree(&Inputs {
                gdbserver: Some(PathBuf::from("target/usr/bin/gdbserver")),
                ..inputs()
            }),
            Err(SystemError::SourceNotAbsolute {
                role: GDBSERVER_PATH,
                path: PathBuf::from("target/usr/bin/gdbserver"),
            })
        );
        assert_eq!(
            tree(&Inputs {
                strace: Some(PathBuf::from("target/usr/bin/strace")),
                ..inputs()
            }),
            Err(SystemError::SourceNotAbsolute {
                role: STRACE_PATH,
                path: PathBuf::from("target/usr/bin/strace"),
            })
        );
    }

    /// Neither is a login shell, and **[`LOGIN_SHELLS`] is the only thing that
    /// makes that true.**
    ///
    /// Written first as `check_login_shells(&t, &[GDBSERVER_PATH]).is_err()`,
    /// which **failed**, and the failure is the finding worth keeping:
    /// [`check_login_shells`] asks *is at least one of these candidates a
    /// present executable*, and a `gdbserver` on the card is one. The guard
    /// knows nothing about what a shell is and was never meant to — its job is
    /// to catch an image whose candidates are all missing, which is the failure
    /// that reaches a board.
    ///
    /// So the property that actually protects `/etc/shells` from naming a debug
    /// stub is the *content of the constant*, and this test asserts exactly
    /// that and no more. Adding `GDBSERVER_PATH` to `LOGIN_SHELLS` would give an
    /// operator an account that `exec`s a program expecting `--attach` and a
    /// port, and nothing else in the build would object.
    #[test]
    fn a_debugger_is_not_among_the_login_shell_candidates() {
        assert!(!LOGIN_SHELLS.contains(&GDBSERVER_PATH));
        assert!(!LOGIN_SHELLS.contains(&STRACE_PATH));
        // And the shipped list is still the two it should be, so that adding a
        // third goes through a reader rather than through this assertion.
        assert_eq!(LOGIN_SHELLS, rivet_core::etc::LOGIN_SHELL_PATHS);
    }

    #[test]
    fn the_login_shell_refusal_names_the_path_and_the_file_it_would_have_broken() {
        let text = SystemError::NoLoginShellInImage {
            candidates: vec![NOT_IN_THE_IMAGE.to_string()],
        }
        .to_string();
        assert!(text.contains(NOT_IN_THE_IMAGE), "{text}");
        assert!(text.contains("/etc/shells"), "{text}");
    }

    // -- the mount points that are not PID 1's --------------------------------

    #[test]
    fn the_extension_mount_point_is_present_and_empty() {
        let t = built();
        assert!(find(&t, EXT_MOUNT_POINT).is_some_and(Node::is_directory));
        assert!(!t.iter().any(|n| n.path.starts_with("/system/ext/")));
    }

    /// `/state` is mounted by `cmd/rivet-initd` since 2026-08-23, and it is
    /// still not in `stage0_plan` — that plan is the *pseudo*-filesystems, and
    /// `/state` is a real block device with failure modes a tmpfs does not have.
    /// The mount point must be in the image either way: it cannot be created on
    /// a read-only root.
    #[test]
    fn the_state_mount_point_is_in_the_image_because_it_cannot_be_created_later() {
        assert!(!stage0_plan().iter().any(|s| s.target == STATE_MOUNT_POINT));
        assert_eq!(provision(STATE_MOUNT_POINT), Provision::InImage);
        assert!(find(&built(), STATE_MOUNT_POINT).is_some_and(Node::is_directory));
    }

    /// The partition `cmd/rivet-initd` will mount there is the one this crate
    /// formats. Two crates, one number, and it now has one definition.
    #[test]
    fn the_state_partition_number_is_the_one_core_defines() {
        assert_eq!(crate::layout::STATE_PARTITION, 5);
        assert_eq!(
            crate::layout::STATE_PARTITION,
            rivet_core::state::STATE_PARTITION
        );
        let state = crate::layout::partition_table()
            .into_iter()
            .find(|p| p.number == crate::layout::STATE_PARTITION);
        assert!(state.is_some_and(|p| p.label == "state"));
    }

    // -- determinism -----------------------------------------------------------

    /// `ADR-015` §4 signs these artifacts. An image that differs run to run is
    /// one whose signature covers something nobody can reproduce.
    #[test]
    fn the_same_inputs_produce_the_same_tree() {
        assert_eq!(tree(&inputs()), tree(&inputs()));
    }

    #[test]
    fn the_tree_is_sorted_and_holds_no_path_twice() {
        let nodes = built();
        let paths: Vec<&str> = nodes.iter().map(|n| n.path.as_ref()).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted, "the tree is not in sorted order");
        sorted.dedup();
        assert_eq!(sorted.len(), paths.len(), "a path appears twice");
    }

    /// The property that makes the order an interface: a caller may walk the
    /// list creating entries as it goes and never has to create a parent.
    #[test]
    fn the_order_creates_a_parent_before_its_children() {
        let t = built();
        for (i, node) in t.iter().enumerate() {
            let Some(parent) = Path::new(node.path.as_ref()).parent() else {
                continue;
            };
            let Some(parent) = parent.to_str() else {
                continue;
            };
            let at = t.iter().position(|n| n.path == parent);
            assert!(
                at.is_some(),
                "{}'s parent {parent} is not in the tree",
                node.path
            );
            assert!(
                at.is_none_or(|at| at < i),
                "{} appears before its parent {parent}",
                node.path
            );
        }
    }

    #[test]
    fn every_path_is_absolute() {
        for node in built() {
            assert!(node.path.starts_with('/'), "{} is not absolute", node.path);
        }
    }

    // -- refusals --------------------------------------------------------------

    /// A relative source resolves against the build process's working
    /// directory, so the same command in two directories builds two different
    /// images. Refused rather than canonicalised — `AGENTS.md` §5.
    #[test]
    fn a_relative_binary_path_is_refused_rather_than_resolved() {
        let bad = Inputs {
            init: PathBuf::from("target/release/rivet-initd"),
            ..inputs()
        };
        assert_eq!(
            tree(&bad),
            Err(SystemError::SourceNotAbsolute {
                role: INIT_PATH,
                path: PathBuf::from("target/release/rivet-initd"),
            })
        );

        let bad = Inputs {
            supervisor: PathBuf::from("./rivet-sysd"),
            ..inputs()
        };
        assert!(matches!(
            tree(&bad),
            Err(SystemError::SourceNotAbsolute {
                role: SUPERVISOR_PATH,
                ..
            })
        ));

        // The two added 2026-08-23. Checked because the check became a loop and
        // a loop that skipped one would still pass the two tests above.
        let bad = Inputs {
            cli: PathBuf::from("rivet"),
            ..inputs()
        };
        assert!(
            matches!(
                tree(&bad),
                Err(SystemError::SourceNotAbsolute { role: CLI_PATH, .. })
            ),
            "a relative CLI path was accepted"
        );
        let bad = Inputs {
            netd: PathBuf::from("../rivet-netd"),
            ..inputs()
        };
        assert!(
            matches!(
                tree(&bad),
                Err(SystemError::SourceNotAbsolute {
                    role: NETD_PATH,
                    ..
                })
            ),
            "a relative netd path was accepted"
        );
        // And an input that comes out of somebody else's build tree, which is
        // the likeliest of all of them to be given relatively: the natural
        // thing to type is the path Buildroot printed, and Buildroot prints
        // relative paths.
        let bad = Inputs {
            http_backend: PathBuf::from("output/usr/libexec/git-core/git-http-backend"),
            ..inputs()
        };
        assert!(
            matches!(
                tree(&bad),
                Err(SystemError::SourceNotAbsolute {
                    role: HTTP_BACKEND_PATH,
                    ..
                })
            ),
            "a relative git-http-backend path was accepted"
        );
    }

    #[test]
    fn the_refusal_names_the_path_and_what_it_was_for() {
        let e = SystemError::SourceNotAbsolute {
            role: INIT_PATH,
            path: PathBuf::from("target/release/rivet-initd"),
        };
        let text = e.to_string();
        assert!(text.contains(INIT_PATH), "{text}");
        assert!(text.contains("target/release/rivet-initd"), "{text}");
    }
    /// **An image that trusts nobody is refused at build time.**
    ///
    /// `ADR-028` §5.5.5, ruled 2026-08-24. The alternative is not "a slightly
    /// worse image" — it is a card that cannot verify an update and therefore
    /// cannot be updated safely, discovered on a board, later, by somebody
    /// trying to fix something else.
    #[test]
    fn an_image_that_would_trust_nobody_is_refused_unless_it_says_so() {
        let mut empty = inputs();
        empty.trust = TrustStore::new(Vec::new());
        empty.development_only = false;
        assert_eq!(tree(&empty), Err(SystemError::NoTrustedKeys));

        // The refusal has to say what to do about it, not merely that it
        // happened — the operator is at a build machine and can still act.
        let said = SystemError::NoTrustedKeys.to_string();
        assert!(said.contains("--trusted-key"), "{said}");
        assert!(said.contains("--no-trust"), "{said}");
    }

    /// The escape hatch is real, and takes a word.
    #[test]
    fn a_development_image_may_trust_nobody_when_it_is_asked_for_deliberately() {
        let mut empty = inputs();
        empty.trust = TrustStore::new(Vec::new());
        empty.development_only = true;
        let t = tree(&empty).expect("--no-trust is a supported card");
        // The file is still there, and still parses — as an empty store, which
        // is what an honest "this machine trusts nobody" looks like.
        let node = find(&t, TRUST_STORE_PATH).expect("the store is in the image regardless");
        let Content::Generated(bytes) = &node.content else {
            panic!("the trust store must be generated, not copied");
        };
        assert!(
            TrustStore::from_bytes(bytes)
                .expect("an empty store is a legal file")
                .is_empty()
        );
    }

    /// What the image carries is what the verifier will read back.
    #[test]
    fn the_trust_store_in_the_image_round_trips_to_what_was_asked_for() {
        let t = tree(&inputs()).expect("the default fixture trusts one key");
        let node = find(&t, TRUST_STORE_PATH).expect("the trust store is in the image");
        assert_eq!(
            node.mode, 0o444,
            "the store is read by everyone, written by nobody"
        );
        let Content::Generated(bytes) = &node.content else {
            panic!("the trust store must be generated");
        };
        let back = TrustStore::from_bytes(bytes).expect("the image wrote a readable store");
        assert_eq!(back.entries().len(), 1);
        assert_eq!(back.entries()[0].label, "rivet-system");
        assert!(
            back.entries()[0]
                .may_sign
                .permits(&rivet_core::package::Kind::update())
        );
        assert!(
            !back.entries()[0]
                .may_sign
                .permits(&rivet_core::package::Kind::application()),
            "the fixture's system key must not be entitled to sign applications"
        );
    }

    /// **The image records its architecture**, where the store's door and the
    /// installer read it, in the form they read — `PLAN-Feature-Set-2026-09-12`
    /// §3.3g. Without it a machine cannot say which artifacts it executes and
    /// admits none.
    #[test]
    fn the_image_records_the_architecture_it_was_built_for_where_the_machine_reads_it() {
        let t = tree(&inputs()).expect("the default fixture");
        let node = find(&t, ARCH_PATH).expect("the architecture record is in the image");
        let Content::Generated(bytes) = &node.content else {
            panic!("the architecture record must be generated, not copied");
        };
        assert_eq!(
            rivet_core::package::arch::read::<&str>(Ok(bytes.clone())),
            Ok("aarch64".to_string())
        );
        assert_eq!(node.mode, 0o444, "read by everyone, written by nobody");
    }

    /// **A token that is not one is refused at build time**, and `arm` — the
    /// compiler's word for an `armv6` board — is the case worth naming.
    #[test]
    fn an_image_told_an_architecture_that_is_not_a_token_is_refused() {
        for bad in ["arm", "", "AARCH64", "aarch64\n"] {
            assert_eq!(
                tree(&Inputs {
                    arch: bad.to_string(),
                    ..inputs()
                }),
                Err(SystemError::UnknownArch(bad.to_string())),
                "{bad:?}"
            );
        }
    }
}
