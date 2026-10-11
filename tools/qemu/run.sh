#!/bin/sh
# Boot a Rivet PID 1 under QEMU, optionally together with the supervisor it
# spawns. See docs/ADR-026 — in particular §4, which says what this loop can and
# cannot settle. It is not a substitute for the board; it is what makes reaching
# the board worth doing.
#
# Usage:
#   ./run.sh <init-binary> [options] [qemu args...]
#
# Options:
#   --supervisor PATH      Also pack PATH into the initramfs, at the path
#                          cmd/rivet-initd compiled into SUPERVISOR_PATH — read
#                          out of the crate below rather than written here, so
#                          this harness cannot drift away from the binary it is
#                          testing.
#
#                          Optional, and deliberately so. Without it PID 1 finds
#                          nothing to spawn and runs its *failure* path — ten
#                          retries with doubling backoff, then GIVING UP and
#                          withholding the boot confirmation. That is correct
#                          behaviour and worth being able to reproduce, so the
#                          one-binary invocation is unchanged.
#
#   --cli PATH             Also pack PATH at the path the runtime /etc
#                          projection probes for a login shell —
#                          rivet_core::etc::CLI_PATH, read out of
#                          rivet-core below rather than written here, for the
#                          same reason SUPERVISOR_PATH is read out of
#                          cmd/rivet-initd.
#
#                          This is the only way to make that probe answer yes
#                          under this emulator. Without it the projection looks,
#                          finds nothing, and writes an /etc/shells that names no
#                          shell — "permits_a_login":false. That is a true
#                          statement about an initramfs with no CLI in it rather
#                          than a defect, and a card can be in exactly that
#                          state, so it stays reproducible: this option is
#                          optional in precisely the way --supervisor is.
#
#                          What it does NOT do is RUN the CLI. Nothing execs it —
#                          PID 1 spawns the supervisor and nothing else. Packing
#                          it settles that the binary is FOUND where the
#                          projection looks; it settles nothing about what
#                          happens when a login actually starts it.
#
#   --nushell PATH         Also pack PATH at rivet_core::etc::NUSHELL_PATH,
#                          read out of the same file as CLI_PATH and for the
#                          same reason.
#
#                          It is the **first** entry of LOGIN_SHELL_PATHS
#                          (the dual-login-shell ruling, ruled 2026-08-24), so packing it
#                          changes two things at once and the console shows
#                          both: `/etc/passwd`'s seventh field becomes nu, and
#                          `/etc/shells` names nu *and* the CLI. Without it the
#                          probe finds only the CLI, which is the card every
#                          build before that date produced and which the ruling
#                          that makes nu optional requires to keep working — so both directions
#                          stay reachable here, exactly as they do for --cli.
#
#                          Nothing execs it, for the same reason nothing execs
#                          the CLI: PID 1 spawns the supervisor and nothing
#                          else. What this settles is that the projection FINDS
#                          it and prefers it. `tools/nushell/build.sh` produces
#                          the binary.
#
#   --netd PATH            Also pack PATH at cmd/rivet-image's NETD_PATH, the
#                          path a card carries it at. The supervisor's
#                          system_services() has started it unconditionally
#                          since 2026-08-23, so this puts the binary where the
#                          supervisor already looks for it.
#
#   --timed PATH           Also pack PATH at cmd/rivet-image's TIMED_PATH.
#                          Added 2026-08-24, and it is the first time
#                          `rivet-timed` has been reachable in this loop at
#                          all: the supervisor has defined the service since
#                          2026-08-23 and every run without this option shows it
#                          REFUSED with ENOENT, which is a true statement about
#                          an initramfs that does not contain it and a poor
#                          default for a harness.
#
#                          Like --mcpd, it is RUN. What it exercises that
#                          nothing else does is ADR-022 §4's FIRST capture path:
#                          `rivet-timed` emits proper §3 records on stdout, so
#                          the supervisor's capture parses them back and
#                          forwards them as records rather than wrapping them as
#                          text — severity, fields and all. Without this option
#                          that branch has no producer.
#
#                          It will not reach an NTP server under -M virt without
#                          --net, and ADR-021 §4's client failing to reach one is
#                          itself worth watching: it is the ordinary state of a
#                          board whose network is not up yet.
#
#   --mcpd PATH            Also pack PATH at cmd/rivet-image's MCPD_PATH — the
#                          control plane (`ADR-035` phase 1), and **the only
#                          route into a guest this harness has** since `ADR-035`
#                          §13 left it as the only way in.
#
#                          Like --timed, it is RUN: the
#                          supervisor's `mcpd` definition is `enabled: true` and
#                          starts it with `--document`, `--identity` and
#                          `--peers` (cmd/rivet-sysd's definitions.rs). Without
#                          this option that spawn REFUSES with ENOENT and the
#                          supervisor says so once, naming the service — which
#                          is a true statement about an initramfs that does not
#                          contain it, and is what every run before 2026-08-30
#                          produced.
#
#                          **Packing it is not the same as being able to reach
#                          it**, and the banner below says which of the three
#                          remaining conditions this run met:
#                            * --secrets, or `rivet-mcpd` cannot read the
#                              identity the supervisor points it at and exits 2
#                              before binding. That is a true statement about a
#                              machine with no identity on /state and stays
#                              reproducible, so it is reported rather than
#                              refused;
#                            * --cli, because what it hands a caller is the
#                              stdin and stdout of `<CLI_PATH> mcp`;
#                            * a `[control-plane] listen` in --document that is
#                              NOT loopback, plus --forward-port, or the
#                              forwarded port reaches nothing. rivet-mcpd's own
#                              default is 127.0.0.1:7777 and SLIRP cannot route
#                              to that.
#
#   --secrets DIR          Place DIR's contents at cmd/rivet-sysd's
#                          MCPD_IDENTITY_DIR — this machine's TLS identity and
#                          the pin file naming who may connect (`ADR-025` §2).
#                          Both paths are read out of that crate below, for the
#                          reason every other path here is.
#
#                          DIR must hold `device.seed`, `device.crt` and the
#                          `peers` file, and a directory missing any of them is
#                          refused: `rivet-mcpd` builds its TLS configuration
#                          BEFORE it binds, so a half identity is a daemon that
#                          exits 2 at start-up and a run that looks like a guest
#                          which never came up.
#
#                          **`cargo rivet identity new` is the only thing that
#                          mints one**, and nothing in this repository holds key
#                          material (`AGENTS.md` §8.3) — so a caller mints a
#                          throwaway pair into a temporary directory, exactly as
#                          `tools/qemu/smoke.sh` does on every run.
#
#                          **Refused together with --card**, for --state's
#                          reason: PID 1 mounts the card over /state and nothing
#                          placed here would be visible after the mount.
#
#   --document PATH        Also pack PATH as this machine's stored
#                          desired-state document, at the path
#                          rivet_core::state::CONFIG_DIR and
#                          rivet_store::MACHINE_LAYER_FILE compose — both read
#                          out of their crates below.
#
#                          It stands in for something QEMU cannot do. On a card,
#                          PID 1 reads the provisioning file off the FAT boot
#                          partition and stores it (ADR-004 §6, ADR-017 §3.1.9);
#                          under -M virt there is no boot partition and no
#                          /dev/mmcblk0, so a machine here always has NO stored
#                          document and rivet-netd exits 2 on every pass.
#                          That is a faithful reproduction of an unprovisioned
#                          board and it makes the interesting half of Stage 1 —
#                          an address, and therefore a reachable control plane
#                          — unreachable in this loop.
#
#                          qemu-network.toml in this directory is the one that
#                          matches QEMU's own user-mode network. It is a HARNESS
#                          fixture, not a sample configuration: the addresses in
#                          it are SLIRP's fixed ones and belong to nothing real.
#
#                          **Refused together with --card.** This packs the
#                          document into the initramfs under /state, and PID 1
#                          mounts the card over it — so the supervisor would
#                          read the card, find no document, and run its
#                          no-configuration path while this console reported a
#                          document packed. PLAN.md §62.3.
#
#   --card IMAGE           Attach IMAGE as a virtio-blk disk, so the guest has
#                          a real GPT and five real partitions instead of none.
#
#                          This is what `ADR-026` §4's `/state` row was waiting
#                          for, and `PLAN.md` §61.2 is the argument that it was
#                          waiting for the wrong thing. Every result this
#                          harness has produced about `/state` landed on the
#                          initramfs tmpfs, because `-M virt` has no
#                          `/dev/mmcblk0` and PID 1's mount degrades. It does
#                          have a virtio-blk controller, and `ADR-024`'s kernel
#                          has carried CONFIG_VIRTIO_BLK and CONFIG_EFI_PARTITION
#                          since it was first built — so an image
#                          `cmd/rivet-image` wrote can simply be handed to it.
#
#                          What a run with this settles:
#                            * the kernel parses the GPT `cmd/rivet-image`
#                              wrote and the five partitions enumerate;
#                            * an ext4 that tool made can be mounted by the
#                              machine that boots on it, and read back.
#
#                          What it does NOT settle: SD/MMC timing, card wear,
#                          and the firmware. `ADR-026` §4's `/state` row narrows
#                          to those three. It does not close.
#
#                          The disk appears as `/dev/vda`, NOT `/dev/mmcblk0`,
#                          and its partitions carry no `p` before the number.
#                          That is not an emulator quirk to be papered over — a
#                          board booted from USB would say `/dev/sda` for the
#                          same reason — and what PID 1 does about it is
#                          `PLAN.md` §62.
#
#                          **The file is not written.** QEMU's `snapshot=on`
#                          puts every guest write in a temporary overlay it
#                          discards on exit, so a 2 GiB build artifact is not
#                          consumed by a run that boots onto it.
#
#   --card-rw              Attach --card's image FOR WRITING, with no discard
#                          overlay. The guest's writes land in the file and
#                          outlive the run.
#
#                          This is the only way to ask whether anything on
#                          `/state` SURVIVES, because one invocation of this
#                          script is one boot: persistence can only be observed
#                          by a second run pointed at the same file. It is
#                          opt-in because the alternative is a harness that
#                          quietly rewrites a named artifact, and `ADR-027`
#                          §3.2 makes that name a claim about its contents.
#
#                          Copy the image first if the original matters.
#
#   --state DIR            Pre-seed the initramfs `/state` from DIR.
#
#                          **Every other run this harness makes starts with an
#                          empty `/state`**, because QEMU `-M virt` has no
#                          `/dev/mmcblk0`, PID 1's mount degrades, and the tmpfs
#                          underneath begins with nothing in it. That means the
#                          only boot ever exercised here is a machine's FIRST —
#                          and a convergence that is correct on an empty
#                          directory and wrong on a populated one would never be
#                          seen. `PLAN.md` §56 is a board in exactly that state.
#
#                          So this option is how *boot 2* is reached: put a
#                          `/state` in DIR with whatever is being tested —
#                          directories with the wrong modes, a key file that is
#                          already there — and the machine boots onto it.
#
#                          It does NOT make anything persist. The tmpfs is still
#                          a tmpfs and still dies with the guest; what this
#                          controls is what it starts with.
#
#                          **Refused together with --card**, because PID 1
#                          mounts the card over this directory and nothing
#                          seeded here would be visible to anything that looks
#                          after the mount. --card --card-rw booted twice is
#                          what this option was imitating, done for real.
#
#   --git PATH             Also pack PATH at rivet_core::git::BINARY. This is
#                          the vendored git the card carries — ONE binary and
#                          not the git-core directory beside it (PLAN.md §43.6)
#                          — and `git-http-backend` runs it by that name.
#
#                          It is RUN twice over, by two different things: the
#                          supervisor runs `git init --bare` to converge
#                          `[repos]`, and `git-http-backend` runs `git` rather
#                          than reaching upload-pack by --exec-path. Without it
#                          a clone over the control plane serves an EMPTY
#                          repository instead of failing — which is a true
#                          statement about an initramfs with no git in it, and
#                          stays reproducible.
#
#   --git-http-backend PATH
#                          Also pack PATH at rivet_core::git::HTTP_BACKEND —
#                          git's smart-HTTP CGI, which rivet-mcpd execs for
#                          every git request on the control plane (ADR-035 §4).
#                          A SECOND binary and not a builtin of --git's
#                          (rivet_core::git's own note), so --git alone packs no
#                          server: a request reaches rivet-mcpd and is answered
#                          `cannot run /system/bin/git-http-backend`. Pair it
#                          with --git, which it runs by name. Added 2026-09-14
#                          for tools/qemu/adr-035-git-proof.sh.
#
#                          **Both may be DYNAMIC, and then --libraries is
#                          required.** Buildroot's `BR2_SHARED_STATIC_LIBS` lets
#                          each package choose, and git chooses shared: it
#                          NEEDs libz.so.1 and the musl loader, as the card that
#                          carries it does (tools/card/build.sh). A dynamic one
#                          with no --libraries is refused by name, because an
#                          initramfs with no loader would turn every git request
#                          into an exec failure that looks like a git one.
#
#   --gdbserver PATH       Also pack PATH at cmd/rivet-image's
#                          GDBSERVER_PATH, read out of that crate below for the
#                          same reason NETD_PATH is: the image crate is the only
#                          copy of the fact, and imitating where a card puts a
#                          binary is the whole point of packing one here.
#
#                          Like --cli and unlike --timed, NOTHING RUNS IT.
#                          There is no service definition and there should not
#                          be — a debug stub started by the supervisor is a
#                          listening socket on every board (ADR-028), and
#                          ADR-005 §7.3 has it started by an operator who has
#                          already logged in. Under this harness that means
#                          asking the machine to run `/system/bin/gdbserver`,
#                          which needs `nu` packed, because cmd/rivet refuses to
#                          exec anything but itself (ADR-005 §7.3). Nothing in
#                          this repository drives one today.
#
#   --strace PATH          Also pack PATH at cmd/rivet-image's STRACE_PATH,
#                          on the same terms. Also run by nobody.
#
#   --bluetoothd PATH      Also pack PATH at cmd/rivet-image's BLUETOOTHD_PATH.
#
#                          The supervisor DEFINES the service and `ADR-041` §6.5
#                          ships it disabled, so packing it is not enough to make
#                          it run: a `--document` with `[services.
#                          rivet-bluetoothd] enabled = true` is what starts it,
#                          which is the same gesture §8.3.1 records on the board.
#
#                          On its own it finds no controller and — correctly —
#                          waits for one rather than failing (`ADR-041` §6.4),
#                          which is a state worth being able to boot. Paired with
#                          --vhci below there IS one.
#
#   --proxyd PATH          Also pack PATH at cmd/rivet-image's PROXYD_PATH —
#                          `ADR-060`'s front door, an image service that runs
#                          under its own uid and Landlock ruleset. Defined and
#                          DISABLED like bluetoothd: a `--document` with
#                          `[services.rivet-proxyd] enabled = true` starts it.
#
#   --vhci PATH            Pack PATH — tools/qemu/vhci-fixture — and boot it as
#                          `rdinit=`, so that this guest has a **Bluetooth
#                          controller**. `ADR-041` §8.4;
#                          tools/buildroot/linux/qemu-virt.config's
#                          CONFIG_BT_HCIVHCI block is what makes it possible and
#                          this is its first caller.
#
#                          Unlike every other binary here it is neither merely
#                          packed nor started by the supervisor: it starts one
#                          child of itself and then **execs the real PID 1**, so
#                          rivet-initd is still process 1 and still reached with
#                          the argv it would have had. What the guest gains is
#                          one extra process, which opens /dev/vhci, writes
#                          HCI_VENDOR_PKT, and IS the controller the kernel then
#                          registers.
#
#                          It is the only way an advertisement can exist on a
#                          machine with no radio, and it settles nothing about a
#                          radio: `ADR-026` §4's right-hand column, and every
#                          byte it delivers was composed by the fixture.
#
#                          Refused together with --supervisor-as-init, which is
#                          the other option that decides what `rdinit=` is.
#
#   --vhci-plan PLAN       What that controller does once the scan is on, as
#                          comma-separated key=value with no spaces — it travels
#                          on the kernel command line. `count=` advertisements,
#                          `gap_us=` apart, after `delay_ms=`, having waited
#                          `wait_s=` for devtmpfs. The fixture's own `Plan` is
#                          the list; the default injects NOTHING, which is a
#                          controller that is present, is found, and hears
#                          nothing.
#
#   --forward-port H:G     Forward host port H to guest port G. Implies --net,
#                          and may be given more than once.
#
#                          **This is how anything reaches the guest.** SLIRP has
#                          no route from the host into the guest without an
#                          explicit hostfwd, so without one the guest's control
#                          plane is reachable from nothing.
#
#                          It replaced a single-port option that had one number
#                          written into it, deleted 2026-08-30 (`ADR-035`
#                          §13). Nothing was lost — `--forward-port 2222:22` is
#                          what that option did — and
#                          what was gained is one option instead of two that
#                          both compose a hostfwd.
#
#   --supervisor-as-init   Boot the supervisor AS pid 1, instead of
#                          rivet-initd. Only for the case below.
#
#   --supervisor-args STR  Arguments for the supervisor, split on spaces.
#                          Requires --supervisor-as-init, and the reason is a
#                          fact about PID 1 rather than a limitation of this
#                          script: `Kernel::spawn` in cmd/rivet-initd takes a
#                          path and nothing else, so a normally-spawned
#                          supervisor gets an empty argv and there is no route
#                          for `--dev-service` (or any other flag) through it.
#                          Booting the supervisor directly is the only way to
#                          watch it supervise anything under this emulator, and
#                          it is NOT the pid1-spawns-supervisor path — the
#                          banner below says so, so that a console captured this
#                          way cannot be mistaken for one that is.
#
#   --net                  Attach a virtio network interface on virtio-mmio,
#                          behind QEMU's user-mode network stack. Stage 1.
#   --net-pci              The same NIC on the generic PCI host bridge instead.
#                          Two transports because the *kernel* has to have a
#                          driver for whichever one is used, and a kernel that
#                          has one and not the other produces a QEMU that starts
#                          cleanly and a guest with no interface — a silence
#                          that looks like a Rivet bug and is not.
#
# ## Networking needs Rivet's own kernel, and this script checks
#
# The Raspberry Pi's own `kernel8.img` binds NOTHING under `-M virt` — verified
# 2026-08-23 across virtio-mmio, virtio-pci and USB-ethernet, and consistent
# with the earlier finding recorded in this directory's README that `strings`
# over the decompressed image contains no `virtio` at all. QEMU will still
# create the device, the kernel will still boot, and the interface will simply
# not be there. That failure is expensive precisely because it is quiet, so
# `--net` looks inside the kernel image first and refuses when it can prove the
# driver is absent. See `docs/ADR-024` §3 and `tools/buildroot/`.
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

usage() {
    cat >&2 <<'EOF'
usage: run.sh <init-binary> [options] [qemu args...]

  --supervisor PATH      also pack PATH into the initramfs, at the path
                         cmd/rivet-initd compiled into SUPERVISOR_PATH.
                         Optional: without it PID 1 runs its failure path —
                         ten retries with doubling backoff, then GIVING UP.
  --cli PATH             also pack PATH at rivet_core::etc::CLI_PATH,
                         which is where the /etc projection probes for a login
                         shell. Optional: without it the probe finds nothing and
                         the run reports "permits_a_login":false, which is a
                         true statement about this initramfs and not a defect.
                         Nothing execs the CLI either way.
  --nushell PATH         also pack PATH at rivet_core::etc::NUSHELL_PATH,
                         which is the FIRST entry of LOGIN_SHELL_PATHS — so
                         with it, root's shell in /etc/passwd is nu and
                         /etc/shells names both (the dual-login-shell ruling). Without it
                         the probe finds only the CLI, which is every card
                         built before 2026-08-24. Nothing execs nu either.
  --netd PATH            also pack PATH at cmd/rivet-image's NETD_PATH.
                         The supervisor starts it (ADR-017 §4.3).
  --timed PATH           also pack PATH at cmd/rivet-image's TIMED_PATH.
                         The supervisor starts it (ADR-021 §7). It is the only
                         service that emits ADR-022 §3 RECORDS rather than
                         lines, so it is what exercises §4's first capture
                         path. Without it every run shows it REFUSED, which is
                         true of an initramfs that lacks it and is a poor
                         default for a harness.
  --mcpd PATH            also pack PATH at cmd/rivet-image's MCPD_PATH, the
                         CONTROL PLANE (ADR-035). The supervisor RUNS it and it
                         is the only route into a guest this harness has — see
                         --secrets and --forward-port, which it needs. Without
                         it the spawn REFUSES with ENOENT and the supervisor
                         says so once, which is true of this initramfs.
  --secrets DIR          place DIR at cmd/rivet-sysd's MCPD_IDENTITY_DIR: this
                         machine's device.seed, device.crt and the peers file
                         (ADR-025 §2). `cargo rivet identity new` mints one;
                         nothing here is committed (AGENTS.md §8.3). A
                         directory missing any of the three is refused, because
                         rivet-mcpd reads them before it binds and a half
                         identity exits 2 at start-up.
  --document PATH        also pack PATH as the stored desired-state document.
                         QEMU has no boot partition, so nothing here can seed
                         one the way PID 1 does on a card; without this the
                         machine has no address and nothing can reach its
                         control plane. qemu-network.toml in this directory
                         matches QEMU's own user-mode network.
  --deploy PATH          also pack PATH at rivet_core::deploy::PROGRAM_PATH,
                         the installer. NOTHING STARTS IT on its own — only a
                         request does, which is what --deploy-request below
                         writes. Without both, a request produces a service
                         instance whose program is not in the image, and the
                         supervisor refuses it once and says so.
  --deploy-request DIGEST
                         also pack an install request naming DIGEST — 64
                         lowercase hex characters — at
                         rivet_core::deploy::REQUEST_PATH, per the maintainer's
                         2026-08-24 install-request ruling: the supervisor looks for one on every
                         convergence pass, removes it, and starts the installer
                         once.

                         **A DIGEST since ADR-052 §7.2, and it used to be a
                         path.** The supervisor resolves it in this machine's
                         artifact store, so this flag also packs a PLACEHOLDER
                         object at
                         /state/artifacts/objects/DIGEST/placeholder.rvt —
                         otherwise every run would exercise the store's refusal
                         rather than the channel. The placeholder is a few bytes
                         of text and its contents are NOT the digest: nothing in
                         this path hashes anything, and nothing opens the file —
                         --deploy packs an installer that WOULD open it, and an
                         installer handed a line of text refuses it.

                         **So this flag settles the SUPERVISOR's half and only
                         that.** For the other half — a real signed .rvt in the
                         store, verified out of it by cmd/rivet-deploy — run
                         tools/qemu/adr-052-proof.sh, which puts artifacts there
                         the way a machine really gets them: over the control
                         plane's bulk endpoint.

                         **It is not an install and must not be reported as
                         one.** It leaves behind what `rivet apply install
                         <digest> confirm` would leave behind, by a different
                         route. What a run with it settles is what the
                         SUPERVISOR does with a request on a real kernel — that
                         it finds it, parses it, RESOLVES it in the store,
                         consumes it and converges — and nothing about the
                         installer, which is not packed by this script at all.

                         The request also says WHO ASKED (ADR-028 §4.5), and
                         what it says by default is the truth about this route:
                         no session at all, because a shell script on the
                         development host wrote it. --deploy-from below is how
                         to write the other branch.
  --deploy-from TOKEN    write TOKEN as the request's principal instead of the
                         default. An `address:port` exercises the branch an
                         `rivet apply install` over the control plane produces;
                         anything
                         rivet_core::deploy will not accept exercises the
                         supervisor's refusal, which is the other thing worth
                         watching on a real kernel.

                         **Nothing authenticates it and nothing here pretends
                         otherwise.** This flag writes a claim into a file, which
                         is exactly what an attacker with root on /state can also
                         do — see ADR-028 §4.5. It is here to reach the code
                         path, never to demonstrate an authorisation.
  --deploy-key LABEL     also write LABEL as the request's key label, which on a
                         real machine is the control-plane peer that
                         authenticated. Same caveat as --deploy-from.
  --deploy-raw BYTES     write BYTES verbatim (plus one newline) at the request
                         path, instead of composing a request. The way to watch
                         the supervisor REFUSE a file that is not a request, on
                         a real kernel — an empty one, a bare path (the format
                         from before ADR-028 §4.5), a field this build does not
                         know. Mutually exclusive with --deploy-request.
  --app NAME:BIN:MANIFEST[:HEADER]
                         install BIN and MANIFEST as an application called
                         NAME, at the paths rivet_core::service names —
                         /state/app/NAME/current/bin/NAME and .../manifest.toml.
                         Repeatable.

                         HEADER, if given, is placed at
                         rivet_core::package::install::HEADER_NAME. Without it
                         the supervisor DEFINES the service and refuses every
                         launch — "no signed header ... so nothing can be
                         verified" (ADR-015 §4.2) — which is correct and is
                         also a machine nothing can be observed running on. Pair
                         it with --trust; tools/qemu/fixture-signer writes both.

                         **It is not an install and must not be reported as
                         one.** ADR-015 §7's install unpacks a signed .rvt,
                         verifies it and swaps a symlink; nothing does that yet
                         (ADR-029 §12 item 4 is ruled and unwritten). This
                         leaves behind what an installer WOULD leave behind, by
                         a different route and with no signature anywhere near
                         it. What a run with it settles is what the SUPERVISOR
                         does with an installed application — ADR-017 §4's
                         second origin, ruled 2026-08-24 — and nothing about
                         installing one.

                         `current` is a real directory here rather than the
                         symlink ADR-015 §7 swaps, for the same reason /state
                         is a tmpfs under this emulator: the harness imitates
                         the shape and settles nothing about the mechanism.
  --font PATH            pack PATH at rivet_core::etc::SYSTEM_FONT, imitating
                         the image's shared typeface exactly as /system/bin
                         imitates its binaries.

                         **For ADR-042 §6's proof and not for drawing.** There
                         is no display here. What it makes testable is the FIRST
                         HALF: rivet_core::confinement grants
                         SYSTEM_FONT_DIR and nothing else under /system, and a
                         probe that found no file there would report ENOENT,
                         which cannot tell a working grant from a broken one.
                         assets/fonts/Geist.ttf is the file the image ships.
  --libraries DIR        pack every *.so* in DIR into /lib, and the musl loader
                         with them, so a DYNAMICALLY LINKED application can run.

                         **Nothing here was dynamic until Mesa.** ADR-003 §5
                         makes every Rivet binary static and this harness
                         refuses a non-static one by name — "there is no loader
                         in the initramfs" — which was true and is the reason
                         examples/probes/mesa-demo and deploy/aura/renderer could be
                         built and not smoke-tested. PLAN.md §104.27.

                         DIR is what tools/card/build.sh's collect_libraries
                         stages: files already named by SONAME, so they land at
                         the name a NEEDED entry asks for. The loader is taken
                         from the cross sysroot, where the card build gets it.

                         **It does not give the guest a GPU.** QEMU -M virt has
                         no VC4, so there is no /dev/dri and Mesa will not find
                         a device. What this makes testable is whether a program
                         EXECS and fails cleanly — which is the question a
                         missing library or a wrong SONAME answers badly.

  --trust FILE           place FILE at rivet_core::package::trust_store::PATH,
                         so the supervisor can VERIFY an installed application
                         (ADR-015 §4.2) instead of refusing every one.

                         **A real store, not one this script composes.** The
                         format is rivet-core's and cmd/rivet-image writes it
                         when a card is built; a shell script emitting its own
                         would be a second producer of a security-relevant file
                         (ADR-011). Extract one from a card image instead:

                           dd if=<card>.img of=/tmp/p3 bs=4M \
                              iflag=skip_bytes,count_bytes skip=<p3 off> count=<len>
                           debugfs -R "dump /system/trust /tmp/trust" /tmp/p3

  --state DIR            pre-seed /state from DIR, instead of leaving it empty.
                         This is how a boot onto a machine that has RUN BEFORE is
                         reached; every other run here starts with nothing there.
  --git PATH             also pack PATH at rivet_core::git::BINARY, the vendored
                         git. RUN by the supervisor, and by the control plane's
                         smart-HTTP handler by way of git-http-backend.
  --git-http-backend PATH
                         also pack PATH at rivet_core::git::HTTP_BACKEND, the
                         CGI rivet-mcpd execs for every git request. A second
                         binary, not a builtin of --git; pair the two.
  --gdbserver PATH       also pack PATH at cmd/rivet-image's GDBSERVER_PATH.
                         NOTHING RUNS IT — ADR-005 §7.3 has an operator start
                         it themselves, so a run with this option proves only
                         that the binary is on the machine. Nothing here drives
                         one.
  --strace PATH          also pack PATH at cmd/rivet-image's STRACE_PATH.
                         Also run by nobody.
  --bluetoothd PATH      also pack PATH at cmd/rivet-image's BLUETOOTHD_PATH.
                         The supervisor defines the service and it ships
                         DISABLED (ADR-041 §6.5), so a document that enables it
                         is what makes it RUN. Without this the spawn refuses
                         with ENOENT once and says so, which is true of an
                         initramfs that lacks it. Pair with --vhci, or it finds
                         no adapter and correctly waits for one for ever.
  --proxyd PATH          also pack PATH at cmd/rivet-image's PROXYD_PATH, the
                         image's front door (ADR-060). DISABLED like
                         bluetoothd; a document enabling rivet-proxyd runs it,
                         as its own uid under a Landlock ruleset.
  --vhci PATH           pack tools/qemu/vhci-fixture and boot it as rdinit=,
                         giving this guest a BLUETOOTH CONTROLLER (ADR-041 §8.4,
                         CONFIG_BT_HCIVHCI). It starts one child of itself and
                         execs the real PID 1, so rivet-initd is still process 1;
                         the child opens /dev/vhci and IS the controller.
                         Settles nothing about a radio. Refused with
                         --supervisor-as-init, which also decides rdinit=.
  --vhci-plan PLAN       what that controller does, as comma-separated key=value
                         with no spaces: count=, gap_us=, delay_ms=, wait_s=.
                         The default injects nothing.
  --forward-port H:G     forward host port H to guest port G; implies --net,
                         repeatable. SLIRP has no route into the guest without
                         one, so this is how the control plane is reached.
  --supervisor-as-init   boot the supervisor AS pid 1 instead of rivet-initd.
  --supervisor-args STR  arguments for the supervisor, split on spaces.
                         Requires --supervisor-as-init: PID 1 spawns its child
                         with an empty argv, so nothing given here could reach
                         a supervisor that PID 1 started.
  --card IMAGE           attach IMAGE as a virtio-blk disk, so /state can be a
                         real ext4 on a real partition table. Guest writes go
                         to a discarded overlay, not to the file.
  --card-rw              attach --card's image for writing instead, so what the
                         guest puts on /state survives into the next run.
  --net                  attach a virtio NIC on virtio-mmio (user networking)
  --net-pci              attach it on the generic PCI host bridge instead

Everything after the last recognised option is passed straight to qemu.
EOF
    exit 2
}

# Look for a virtio-net driver inside a kernel image.
#
# Three answers, not two, and the third is the important one. `AGENTS.md` §7:
# never conclude absence from a single probe, because an error and a true
# negative are indistinguishable. So this returns:
#   0  found      — the driver's strings are in there
#   1  absent     — the image was read successfully and does not contain them
#   2  unknown    — the image could not be read in a form that could be searched
# and the caller refuses only on 1. An "unknown" that refused would make this
# script unusable against a kernel packed in a way it has not been taught about,
# which is a worse failure than the one it is guarding against.
kernel_virtio_net() {
    k=$1

    # Without `strings` every search below returns "no match", which would be
    # read as "absent" and refuse a kernel that is perfectly fine. A missing
    # tool is an error, not a negative result — that distinction is the whole
    # point of the third return value.
    if ! command -v strings >/dev/null 2>&1; then
        return 2
    fi

    # An arm64 `Image` is uncompressed and this finds it directly.
    if strings -a "$k" 2>/dev/null | grep -q 'virtio_net'; then
        return 0
    fi

    # `kernel8.img` is a decompressor stub wrapping a gzip member, so the raw
    # search above proves nothing about it. Find the member and look inside.
    # `grep -P` is GNU-specific; where it is missing the answer is "unknown"
    # rather than "absent".
    #
    # The probe feeds grep something that MUST match. An earlier version fed it
    # an empty pattern on empty input, which exits 1 for having found no lines —
    # so every kernel came back "unknown", the guard never fired once, and the
    # bug was invisible because "unknown" continues rather than failing.
    if ! printf 'x\n' | grep -qP 'x' 2>/dev/null; then
        return 2
    fi
    off=$(grep -abo -P '\x1f\x8b\x08' "$k" 2>/dev/null | head -1 | cut -d: -f1)
    if [ -z "$off" ]; then
        # Not gzip-wrapped either. The image was readable and the strings are
        # not in it, in any form this knows how to unwrap.
        return 1
    fi
    if tail -c "+$((off + 1))" "$k" 2>/dev/null | gzip -dc 2>/dev/null \
        | strings -a | grep -q 'virtio_net'; then
        return 0
    fi
    return 1
}

# Refuse rather than guess: a host binary in the initramfs boots to nothing and
# looks like a hang. AGENTS.md §3 — stubs refuse, they never fake success. Both
# binaries go through this, because a supervisor that is present but unrunnable
# would look exactly like the absent-supervisor failure path and quietly rob the
# run of everything it was meant to demonstrate.
require_static_aarch64() {
    what=$1
    path=$2
    if [ ! -f "$path" ]; then
        echo "$0: no such $what: $path" >&2
        exit 1
    fi
    case $(file -b "$path" 2>/dev/null) in
        *aarch64*static*) ;;
        *aarch64*)
            echo "$0: $what $path is aarch64 but not static; there is no loader in the initramfs" >&2
            exit 1 ;;
        *)
            echo "$0: $what $path is not a static aarch64 binary:" >&2
            file -b "$path" >&2
            exit 1 ;;
    esac
}

# The same, for a vendored C program Buildroot may have linked either way: a
# DYNAMIC aarch64 one is accepted when --libraries packs the loader and the
# libraries it names, and refused by name when it does not.
require_runnable_aarch64() {
    what=$1
    path=$2
    case $(file -b "$path" 2>/dev/null) in
        *aarch64*dynamically*)
            [ -n "${LIBRARIES_DIR:-}" ] && return 0
            echo "$0: $what $path is aarch64 and DYNAMICALLY linked, and no --libraries" >&2
            echo "    was given, so the initramfs has no loader to run it. Pass --libraries DIR" >&2
            echo "    holding the libraries it NEEDs (readelf -d lists them)." >&2
            exit 1 ;;
    esac
    require_static_aarch64 "$what" "$path"
}

# Read an absolute path out of a crate's own source, instead of writing it here.
#
# Every binary this script packs goes somewhere a *different* binary looks for
# it, and each of those destinations is a constant in the crate that does the
# looking. Restating one here would produce a harness that packs a file where
# nothing looks for it the day a ruling moves the path — and the symptom would
# be the failure path running, quietly, in a run that was supposed to
# demonstrate the success path. So the constant is read, and a value that is not
# an absolute path is a refusal rather than a guess (AGENTS.md §5).
#
# `exit 1` inside the command substitution ends only the subshell, so callers
# use `|| exit 1` and the message this prints is what the user sees.
crate_path() {
    what=$1
    file=$2
    name=$3
    if [ ! -f "$file" ]; then
        echo "$0: cannot read $file to learn where the $what belongs" >&2
        exit 1
    fi
    value=$(sed -n "s/^pub const $name: *&str *= *\"\([^\"]*\)\".*/\1/p" "$file" | head -1)

    # **A constant may be an ALIAS for another one**, and this follows exactly
    # one hop. `MCPD_IDENTITY_DIR` became `= rivet_core::state::SECRETS` on
    # 2026-08-30, after which this extractor found no literal, refused, and
    # took the whole control-plane boot of `smoke.sh` down with it — for a day,
    # unnoticed, because the gate's SECOND boot is the only thing that needs it
    # and a red gate reads the same however it went red. `PLAN.md` §104.23.
    #
    # One hop and no more. A chain of aliases is a thing to fix in the source
    # rather than to chase from a shell script, and a resolver that followed
    # arbitrarily far would be a small interpreter living in a test harness.
    if [ -z "$value" ]; then
        alias_target=$(sed -n \
            "s/^pub const $name: *&str *= *[A-Za-z_][A-Za-z0-9_]*\(::[A-Za-z0-9_]*\)*::\([A-Z_][A-Z0-9_]*\) *;.*/\2/p" \
            "$file" | head -1)
        if [ -n "$alias_target" ]; then
            # The alias names a constant in another crate; search the tree for
            # its definition rather than guessing which file holds it. Linked
            # worktrees under .claude/ are whole copies of the tree, possibly
            # stale, and build output is not source: searching either reads
            # tens of gigabytes across the container's bind mount before the
            # guest starts, and could answer with another tree's value.
            value=$(grep -rhs --include='*.rs' \
                --exclude-dir=.claude --exclude-dir=.git --exclude-dir=target --exclude-dir=out \
                "^pub const $alias_target: *&str *= *\"" "$REPO" \
                | sed -n "s/^pub const $alias_target: *&str *= *\"\([^\"]*\)\".*/\1/p" \
                | head -1)
        fi
    fi

    case $value in
        /*) ;;
        *)  echo "$0: no absolute $name found in $file" >&2
            echo "    (found: '$value'). Refusing to guess where the $what goes." >&2
            echo "    If $name is an alias for a constant in another crate, this" >&2
            echo "    follows ONE hop to a string literal and no further." >&2
            exit 1 ;;
    esac
    printf '%s\n' "$value"
}

# crate_path's sibling for a constant that is NOT a path. Same extraction, same
# refusal to guess, without the leading-slash check — the request format's field
# names and reserved words are tokens rather than paths, and requiring a `/` of
# them would refuse every one.
crate_token() {
    what=$1
    file=$2
    name=$3
    if [ ! -f "$file" ]; then
        echo "$0: cannot read $file to learn the $what" >&2
        exit 1
    fi
    value=$(sed -n "s/^pub const $name: *&str *= *\"\([^\"]*\)\".*/\1/p" "$file" | head -1)
    if [ -z "$value" ]; then
        echo "$0: no $name found in $file. Refusing to guess the $what." >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

# Copy one binary into the staging tree at the path it is installed at on a
# card, creating the directory it lives in. ADR-008 §3 puts them all under
# /system/bin, which the initramfs is imitating; nothing else about a real image
# is imitated here, and the comment at the call site for the supervisor says why
# that stops where it does.
pack() {
    src=$1
    dest=$2
    mkdir -p "$ROOTFS$(dirname "$dest")"
    cp "$src" "$ROOTFS$dest"
    chmod +x "$ROOTFS$dest"
}

INIT_BIN=$1
if [ -z "$INIT_BIN" ]; then
    usage
fi
# Without this shift, "$@" below still contains $INIT_BIN, and it gets handed
# to qemu-system-aarch64 a second time as a bare positional argument — which
# qemu treats as an implicit raw disk image to open and lock, silently. That
# doesn't fail outright (the ELF just gets probed as a bogus "drive"), but it
# holds a write lock on the binary for the life of the VM and produces a
# "Failed to get \"write\" lock" error the next time anything tries to read
# or reopen it, e.g. a second run before this one's QEMU has exited.
shift

SUPERVISOR_BIN=
CLI_BIN=
NUSHELL_BIN=
NETD_BIN=
TIMED_BIN=
MCPD_BIN=
SECRETS_DIR=
GIT_BIN=
HTTP_BACKEND_BIN=
FONT_FILE=
STATE_DIR=
GDBSERVER_BIN=
STRACE_BIN=
BLUETOOTHD_BIN=
PROXYD_BIN=
DOCUMENT_FILE=
DEPLOY_REQUEST=
# The request's principal, ADR-028 §4.5. Empty means "whatever rivet-core
# spells `no session at all`", which is the true statement about a request this
# script wrote — see --deploy-from in `usage`.
DEPLOY_FROM=
DEPLOY_KEY=
# `--deploy-raw`, and a separate flag for "it was given" because the empty
# string is a case worth being able to write: a request file that exists and
# says nothing is one of the shapes the supervisor refuses.
DEPLOY_RAW=
DEPLOY_RAW_SET=no
DEPLOY_BIN=
# `--app NAME:BINARY:MANIFEST`, space separated. See the option in `usage`.
APPS=
EXTRA_FORWARDS=
SUPERVISOR_AS_INIT=no
SUPERVISOR_ARGS=
# `--vhci`, and the plan it is given. See the option in `usage`.
VHCI_BIN=
VHCI_PLAN=
UDHCPC_BIN=
DHCP_HOOK_BIN=
NET=no
NET_TRANSPORT=mmio
CARD_IMAGE=
CARD_RW=no
# Options are consumed only until the first thing that is not one of ours;
# everything from there on is qemu's, exactly as before.
while [ $# -gt 0 ]; do
    case $1 in
        --card)
            if [ $# -lt 2 ]; then
                echo "$0: --card needs a path to a card image" >&2
                exit 2
            fi
            CARD_IMAGE=$2
            shift 2 ;;
        --card-rw)
            CARD_RW=yes
            shift ;;
        --net)
            NET=yes
            shift ;;
        --net-pci)
            NET=yes
            NET_TRANSPORT=pci
            shift ;;
        --supervisor)
            if [ $# -lt 2 ]; then
                echo "$0: --supervisor needs a path" >&2
                exit 2
            fi
            SUPERVISOR_BIN=$2
            shift 2 ;;
        --cli)
            if [ $# -lt 2 ]; then
                echo "$0: --cli needs a path" >&2
                exit 2
            fi
            CLI_BIN=$2
            shift 2 ;;
        --nushell)
            if [ $# -lt 2 ]; then
                echo "$0: --nushell needs a path" >&2
                exit 2
            fi
            NUSHELL_BIN=$2
            shift 2 ;;
        --timed)
            if [ $# -lt 2 ]; then
                echo "$0: --timed needs a path" >&2
                exit 2
            fi
            TIMED_BIN=$2
            shift 2 ;;
        --netd)
            if [ $# -lt 2 ]; then
                echo "$0: --netd needs a path" >&2
                exit 2
            fi
            NETD_BIN=$2
            shift 2 ;;
        --udhcpc)
            if [ $# -lt 2 ]; then
                echo "$0: --udhcpc needs a path" >&2
                exit 2
            fi
            UDHCPC_BIN=$2
            # **Implies --net.** A DHCP client with no NIC has nothing to ask,
            # and QEMU's user networking runs a real DHCP server -- which is
            # what makes proving this here worth anything.
            NET=yes
            shift 2 ;;
        --dhcp-hook)
            if [ $# -lt 2 ]; then
                echo "$0: --dhcp-hook needs a path" >&2
                exit 2
            fi
            DHCP_HOOK_BIN=$2
            shift 2 ;;
        --mcpd)
            if [ $# -lt 2 ]; then
                echo "$0: --mcpd needs a path" >&2
                exit 2
            fi
            MCPD_BIN=$2
            shift 2 ;;
        --secrets)
            if [ $# -lt 2 ]; then
                echo "$0: --secrets needs a directory" >&2
                exit 2
            fi
            SECRETS_DIR=$2
            shift 2 ;;
        --trust)
            if [ $# -lt 2 ]; then
                echo "$0: --trust needs a file" >&2
                exit 2
            fi
            TRUST_FILE=$2
            shift 2 ;;
        --libraries)
            if [ $# -lt 2 ]; then
                echo "$0: --libraries needs a directory" >&2
                exit 2
            fi
            LIBRARIES_DIR=$2
            shift 2 ;;
        --state)
            if [ $# -lt 2 ]; then
                echo "$0: --state needs a directory" >&2
                exit 2
            fi
            STATE_DIR=$2
            shift 2 ;;
        --git)
            if [ $# -lt 2 ]; then
                echo "$0: --git needs a path" >&2
                exit 2
            fi
            GIT_BIN=$2
            shift 2 ;;
        --git-http-backend)
            if [ $# -lt 2 ]; then
                echo "$0: --git-http-backend needs a path" >&2
                exit 2
            fi
            HTTP_BACKEND_BIN=$2
            shift 2 ;;
        --font)
            if [ $# -lt 2 ]; then
                echo "$0: --font needs a path" >&2
                exit 2
            fi
            FONT_FILE=$2
            shift 2 ;;
        --gdbserver)
            if [ $# -lt 2 ]; then
                echo "$0: --gdbserver needs a path" >&2
                exit 2
            fi
            GDBSERVER_BIN=$2
            shift 2 ;;
        --strace)
            if [ $# -lt 2 ]; then
                echo "$0: --strace needs a path" >&2
                exit 2
            fi
            STRACE_BIN=$2
            shift 2 ;;
        --bluetoothd)
            if [ $# -lt 2 ]; then
                echo "$0: --bluetoothd needs a path" >&2
                exit 2
            fi
            BLUETOOTHD_BIN=$2
            shift 2 ;;
        --proxyd)
            if [ $# -lt 2 ]; then
                echo "$0: --proxyd needs a path" >&2
                exit 2
            fi
            PROXYD_BIN=$2
            shift 2 ;;
        --vhci)
            if [ $# -lt 2 ]; then
                echo "$0: --vhci needs a path to tools/qemu/vhci-fixture's binary" >&2
                exit 2
            fi
            VHCI_BIN=$2
            shift 2 ;;
        --vhci-plan)
            if [ $# -lt 2 ]; then
                echo "$0: --vhci-plan needs a plan, e.g. count=40,gap_us=0" >&2
                exit 2
            fi
            # **Spaces are refused here rather than in the guest.** The plan
            # travels on the kernel command line, where a space would end it and
            # make the rest of it init's next argument — so the fixture would
            # receive a plan it could parse and a path that is not PID 1, and
            # the machine would not come up. Caught at the shell, it is one
            # sentence; caught in the guest, it is a boot that hangs.
            case $2 in
                *[[:space:]]*)
                    echo "$0: --vhci-plan must not contain spaces: '$2'" >&2
                    exit 2 ;;
            esac
            VHCI_PLAN=$2
            shift 2 ;;
        --forward-port)
            if [ $# -lt 2 ]; then
                echo "$0: --forward-port needs HOST:GUEST" >&2
                exit 2
            fi
            # Validated here rather than handed to QEMU, because QEMU's own
            # refusal for a malformed hostfwd arrives after the boot has been
            # attempted and reads as a device error. Two decimal numbers with
            # one colon; anything else is refused by name.
            case $2 in
                *:*)
                    _fwd_host=${2%%:*}
                    _fwd_guest=${2##*:}
                    case "$_fwd_host$_fwd_guest" in
                        ''|*[!0-9]*)
                            echo "$0: --forward-port '$2' is not HOST:GUEST with two port numbers" >&2
                            exit 2 ;;
                    esac
                    if [ "$_fwd_host" != "${2%:*}" ] || [ "$_fwd_guest" != "${2#*:}" ]; then
                        echo "$0: --forward-port '$2' has more than one colon" >&2
                        exit 2
                    fi
                    ;;
                *)
                    echo "$0: --forward-port '$2' is not HOST:GUEST" >&2
                    exit 2 ;;
            esac
            EXTRA_FORWARDS="$EXTRA_FORWARDS,hostfwd=tcp::$_fwd_host-:$_fwd_guest"
            NET=yes
            shift 2 ;;
        --document)
            if [ $# -lt 2 ]; then
                echo "$0: --document needs a path" >&2
                exit 2
            fi
            DOCUMENT_FILE=$2
            shift 2 ;;
        --deploy)
            if [ $# -lt 2 ]; then
                echo "$0: --deploy needs a path" >&2
                exit 2
            fi
            DEPLOY_BIN=$2
            shift 2 ;;
        --deploy-request)
            if [ $# -lt 2 ]; then
                echo "$0: --deploy-request needs an artifact digest" >&2
                exit 2
            fi
            # **A digest, since ADR-052 §7.2, and it used to be an absolute
            # path.** The check is here for the reason the path check was: the
            # supervisor refuses anything that is not one, so packing a wrong
            # shape would test the refusal by accident and report it as the
            # channel working.
            if ! printf '%s' "$2" | grep -Eq '^[0-9a-f]{64}$'; then
                echo "$0: --deploy-request '$2' is not a BLAKE3 digest — 64 lowercase hex" >&2
                echo "    characters. An install names a digest and the supervisor resolves it" >&2
                echo "    in this machine's artifact store (ADR-052 §7.2); there is no form of a" >&2
                echo "    request that names a path." >&2
                exit 2
            fi
            DEPLOY_REQUEST=$2
            shift 2 ;;
        --deploy-from)
            if [ $# -lt 2 ]; then
                echo "$0: --deploy-from needs a principal token" >&2
                exit 2
            fi
            # **Not validated here, deliberately.** rivet_core::deploy is the
            # one reader of this format and a second opinion in this file is
            # what ADR-011 is about; a token it refuses is a run that watches
            # the supervisor refuse it, which is a thing worth being able to do.
            DEPLOY_FROM=$2
            shift 2 ;;
        --deploy-key)
            if [ $# -lt 2 ]; then
                echo "$0: --deploy-key needs a label" >&2
                exit 2
            fi
            DEPLOY_KEY=$2
            shift 2 ;;
        --deploy-raw)
            if [ $# -lt 2 ]; then
                echo "$0: --deploy-raw needs the bytes to write" >&2
                exit 2
            fi
            DEPLOY_RAW=$2
            DEPLOY_RAW_SET=yes
            shift 2 ;;
        --app)
            if [ $# -lt 2 ]; then
                echo "$0: --app needs NAME:BINARY:MANIFEST" >&2
                exit 2
            fi
            APPS="$APPS $2"
            shift 2 ;;
        --supervisor-as-init)
            SUPERVISOR_AS_INIT=yes
            shift ;;
        --supervisor-args)
            if [ $# -lt 2 ]; then
                echo "$0: --supervisor-args needs a value" >&2
                exit 2
            fi
            SUPERVISOR_ARGS=$2
            shift 2 ;;
        -h|--help)
            usage ;;
        *)
            break ;;
    esac
done

if [ "$SUPERVISOR_AS_INIT" = yes ] && [ -z "$SUPERVISOR_BIN" ]; then
    echo "$0: --supervisor-as-init needs --supervisor <path>; there is nothing to boot otherwise" >&2
    exit 2
fi
if [ -n "$SECRETS_DIR" ] && [ -z "$MCPD_BIN" ]; then
    echo "$0: --secrets without --mcpd would do nothing." >&2
    echo "    Nothing else on a Rivet reads that directory: the identity and the" >&2
    echo "    pin file are arguments the supervisor hands rivet-mcpd, and with no" >&2
    echo "    binary packed that spawn refuses before it opens either. Refusing" >&2
    echo "    rather than accepting a flag that is silently inert." >&2
    exit 2
fi
if [ -n "$SUPERVISOR_ARGS" ] && [ "$SUPERVISOR_AS_INIT" != yes ]; then
    echo "$0: --supervisor-args without --supervisor-as-init would do nothing." >&2
    echo "    PID 1 spawns its child with an empty argv — cmd/rivet-initd's" >&2
    echo "    Kernel::spawn takes a path and no arguments — so nothing given here" >&2
    echo "    could reach a supervisor that PID 1 started. Refusing rather than" >&2
    echo "    accepting a flag that silently does nothing." >&2
    exit 2
fi
if [ -n "$VHCI_BIN" ] && [ "$SUPERVISOR_AS_INIT" = yes ]; then
    echo "$0: --vhci and --supervisor-as-init both decide what rdinit= is." >&2
    echo "    The fixture becomes rdinit= and then execs whatever pid 1 would" >&2
    echo "    have been, so the two are not incompatible in principle — but a" >&2
    echo "    combination nothing has run is not one this script will invent." >&2
    exit 2
fi
if [ -n "$VHCI_PLAN" ] && [ -z "$VHCI_BIN" ]; then
    echo "$0: --vhci-plan without --vhci would do nothing." >&2
    echo "    The plan is read by the fixture, which is not on this machine" >&2
    echo "    unless --vhci packs it. Refusing rather than accepting a flag" >&2
    echo "    that is silently inert." >&2
    exit 2
fi

require_static_aarch64 "init binary" "$INIT_BIN"

# **Where a BOARD puts pid 1**, read out of cmd/rivet-image rather than written
# here, exactly as NETD_PATH and TIMED_PATH are.
#
# This used to be `/init`, and that one choice is what put a whole row in
# ADR-026 4's right-hand column. `/proc/1/comm` is the file name of what the
# kernel exec'd, so packing at `/init` made every Rivet machine under this
# emulator answer `init` - and `cmd/rivet`'s interlock, which requires exactly
# `rivet-initd` before it will signal pid 1, refused every stop and every
# reload. That was read as *the emulator cannot settle what an operator asks
# pid 1 to do*, and it was really *this script packs the binary under the wrong
# name*.
#
# `rdinit=` was already how the kernel is told what to run - the comment below
# says so - so nothing required the name `/init` at all.
INIT_PATH=$(crate_path "pid 1" "$REPO/cmd/rivet-image/src/system.rs" INIT_PATH) || exit 1

SUPERVISOR_PATH=
if [ -n "$SUPERVISOR_BIN" ]; then
    require_static_aarch64 "supervisor binary" "$SUPERVISOR_BIN"

    # PID 1's own constant, and it is deliberately not /sbin — ADR-008 §3 and
    # ADR-016 §3 put the image's binaries under /system/bin, and ADR-002 §4's
    # cmdline.txt is stale on exactly this point (already flagged in
    # supervise.rs).
    SUPERVISOR_PATH=$(crate_path "supervisor" \
        "$REPO/cmd/rivet-initd/src/supervise.rs" SUPERVISOR_PATH) || exit 1
fi

CLI_PATH=
if [ -n "$CLI_BIN" ]; then
    require_static_aarch64 "cli binary" "$CLI_BIN"

    # rivet-core's constant and not cmd/rivet's, because the destination
    # that matters is the one the *projection probes*: cmd/rivet-sysd's
    # etc.rs calls file_exists on each of LOGIN_SHELL_PATHS and writes /etc/shells
    # from the answers. Packing anywhere else would produce a boot that still
    # reports names_a_login_shell: false, which is the exact observation this
    # option exists to change.
    CLI_PATH=$(crate_path "cli" "$REPO/rivet-core/src/etc.rs" CLI_PATH) || exit 1

    # The same fact has a second copy in cmd/rivet — ADR-011 forbids that and
    # the crate's own doc comment says so; a binary crate has no library target,
    # so nothing can import it. Nothing pins the two against each other (the
    # tripwire in cmd/rivet-image compares its CLI_PATH to the literal, not to
    # rivet-core's), so this harness checks them, because it is the one place
    # both are read at once. A disagreement would put the CLI where the
    # projection looks and not where the CLI believes it lives — a console
    # showing a login shell found, on top of two constants that no longer agree.
    CLI_ANSWERS_TO=$(crate_path "cli" \
        "$REPO/cmd/rivet/src/invocation.rs" INSTALL_PATH) || exit 1
    if [ "$CLI_PATH" != "$CLI_ANSWERS_TO" ]; then
        echo "$0: the two copies of the CLI's install path disagree:" >&2
        echo "    rivet_core::etc::CLI_PATH            = $CLI_PATH" >&2
        echo "    cmd/rivet's invocation::INSTALL_PATH = $CLI_ANSWERS_TO" >&2
        echo "    Refusing to pack at one of them. Whichever is stale, a run" >&2
        echo "    made now would report a login shell found at a path the CLI" >&2
        echo "    does not answer to." >&2
        exit 1
    fi
fi

NUSHELL_PATH=
if [ -n "$NUSHELL_BIN" ]; then
    require_static_aarch64 "nushell binary" "$NUSHELL_BIN"

    # The same constant the projection probes, from the same file as CLI_PATH.
    # There is no second copy to cross-check this one against — nushell is
    # somebody else's crate and holds no opinion about where Rivet installs
    # it — so unlike the CLI there is no disagreement to detect, only a path to
    # read from the one place that owns it.
    NUSHELL_PATH=$(crate_path "nushell" "$REPO/rivet-core/src/etc.rs" NUSHELL_PATH) || exit 1
fi

NETD_PATH=
if [ -n "$NETD_BIN" ]; then
    require_static_aarch64 "netd binary" "$NETD_BIN"

    # cmd/rivet-image's constant, because it is the only copy: cmd/rivet-netd
    # holds no install path of its own — checked two ways, `grep -rn 'pub const
    # .*: &str = "/' cmd/rivet-netd/src` and `git grep system/bin/rivet-netd`,
    # which between them find the image crate and a doc comment quoting it
    # (AGENTS.md §7). The image is what puts this on a card, so the image is
    # what this imitates.
    NETD_PATH=$(crate_path "network daemon" \
        "$REPO/cmd/rivet-image/src/system.rs" NETD_PATH) || exit 1
fi

# The DHCP client and the program it execs. Both paths come from
# cmd/rivet-image's constants for NETD_PATH's reason: the image is what puts
# these on a card, so this is what the loop imitates.
#
# **The pairing is enforced here as it is there.** cmd/rivet-image refuses a
# card carrying the client without the hook, because udhcpc would take an offer
# and then exec a path that is not present -- and a guest that does that looks
# exactly like one nothing answered.
UDHCPC_PATH=
DHCP_HOOK_PATH=
if [ -n "$UDHCPC_BIN" ]; then
    require_static_aarch64 "DHCP client" "$UDHCPC_BIN"
    if [ -z "$DHCP_HOOK_BIN" ]; then
        echo "$0: --udhcpc without --dhcp-hook." >&2
        echo "    udhcpc execs a program to record the lease; with none packed it would" >&2
        echo "    take an offer and do nothing, which looks like a network that is down." >&2
        echo "    cmd/rivet-image refuses the same combination for the same reason." >&2
        exit 2
    fi
    UDHCPC_PATH=$(crate_path "DHCP client" \
        "$REPO/cmd/rivet-image/src/system.rs" UDHCPC_PATH) || exit 1
fi
if [ -n "$DHCP_HOOK_BIN" ]; then
    require_static_aarch64 "DHCP hook" "$DHCP_HOOK_BIN"
    DHCP_HOOK_PATH=$(crate_path "DHCP hook" \
        "$REPO/cmd/rivet-image/src/system.rs" DHCP_HOOK_PATH) || exit 1
fi

TIMED_PATH=
if [ -n "$TIMED_BIN" ]; then
    require_static_aarch64 "timed binary" "$TIMED_BIN"

    # cmd/rivet-image's constant, for the same reason NETD_PATH is read from
    # there: it is the only copy, and the image is what puts this on a card.
    TIMED_PATH=$(crate_path "time client" \
        "$REPO/cmd/rivet-image/src/system.rs" TIMED_PATH) || exit 1
fi

BLUETOOTHD_PATH=
if [ -n "$BLUETOOTHD_BIN" ]; then
    require_static_aarch64 "bluetoothd binary" "$BLUETOOTHD_BIN"

    # cmd/rivet-image's constant, for TIMED_PATH's reason. `cmd/rivet-sysd`'s
    # definitions.rs carries a second spelling of the same string and a test
    # that fails if the two ever part; either is the right file to read and
    # this reads the image's, as every other binary here does.
    BLUETOOTHD_PATH=$(crate_path "bluetooth daemon" \
        "$REPO/cmd/rivet-image/src/system.rs" BLUETOOTHD_PATH) || exit 1
fi

PROXYD_PATH=
if [ -n "$PROXYD_BIN" ]; then
    require_static_aarch64 "proxyd binary" "$PROXYD_BIN"
    # An alias for rivet-core's constant, which crate_path follows one hop.
    PROXYD_PATH=$(crate_path "path proxy" \
        "$REPO/cmd/rivet-image/src/system.rs" PROXYD_PATH) || exit 1
fi

GIT_PATH=
if [ -n "$GIT_BIN" ]; then
    require_runnable_aarch64 "git binary" "$GIT_BIN"

    # rivet-core's constant, read by cmd/rivet-sysd to converge `[repos]` and by
    # git-http-backend, which runs `git` rather than reaching upload-pack by
    # --exec-path. One copy, ADR-011.
    GIT_PATH=$(crate_path "git" "$REPO/rivet-core/src/git.rs" BINARY) || exit 1
fi

HTTP_BACKEND_PATH=
if [ -n "$HTTP_BACKEND_BIN" ]; then
    require_runnable_aarch64 "git-http-backend binary" "$HTTP_BACKEND_BIN"
    [ -n "$GIT_BIN" ] || {
        echo "$0: --git-http-backend without --git packs a server that cannot serve:" >&2
        echo "    git-http-backend runs \`git\` by name for upload-pack and receive-pack," >&2
        echo "    and with no git beside it a clone reports an EMPTY repository." >&2
        exit 2
    }
    # rivet-core's constant, read by cmd/rivet-mcpd when it execs the CGI.
    HTTP_BACKEND_PATH=$(crate_path "git-http-backend" "$REPO/rivet-core/src/git.rs" HTTP_BACKEND) || exit 1
fi

# **The system typeface** — `ADR-042` §6, added 2026-09-05.
#
# `rivet_core::confinement` grants `rivet_core::etc::SYSTEM_FONT_DIR` and
# nothing else under `/system`, and the FIRST HALF of that proof needs a real
# file at the granted path: a probe that finds nothing there gets `ENOENT` and
# cannot tell a working grant from a broken one. This is the same imitation
# `/system/bin` already is — the initramfs standing in for image content — and
# it settles nothing about how a card is assembled.
FONT_PATH=
if [ -n "$FONT_FILE" ]; then
    if [ ! -f "$FONT_FILE" ]; then
        echo "$0: no such typeface: $FONT_FILE" >&2
        exit 2
    fi
    FONT_PATH=$(crate_path "the system typeface" \
        "$REPO/rivet-core/src/etc.rs" SYSTEM_FONT) || exit 1
fi

MCPD_PATH=
if [ -n "$MCPD_BIN" ]; then
    require_static_aarch64 "control plane binary" "$MCPD_BIN"

    # cmd/rivet-image's constant, for NETD_PATH's reason: the image is what puts
    # this on a card, and cmd/rivet-sysd's `MCPD_PATH` is a second spelling of
    # the same fact that its own doc comment already apologises for. Packing
    # where the IMAGE puts it is packing where the supervisor's definition
    # looks, because the two agree — and the tripwire for the day they stop is
    # the check below, which is the one place both are read at once.
    MCPD_PATH=$(crate_path "control plane" \
        "$REPO/cmd/rivet-image/src/system.rs" MCPD_PATH) || exit 1
    MCPD_SUPERVISOR_PATH=$(crate_path "control plane" \
        "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PATH) || exit 1
    if [ "$MCPD_PATH" != "$MCPD_SUPERVISOR_PATH" ]; then
        echo "$0: the two copies of the control plane's install path disagree:" >&2
        echo "    cmd/rivet-image's MCPD_PATH   = $MCPD_PATH" >&2
        echo "    cmd/rivet-sysd's MCPD_PATH    = $MCPD_SUPERVISOR_PATH" >&2
        echo "    Refusing to pack at one of them. Whichever is stale, a run made" >&2
        echo "    now would put the binary where the card puts it and have the" >&2
        echo "    supervisor look somewhere else — or the reverse." >&2
        exit 1
    fi
fi

# Where the identity and the pin file go. Read out of cmd/rivet-sysd, because
# the argument vector the supervisor hands `rivet-mcpd` is what decides, and
# that vector is written there — `ADR-017` §4.1 keeps it the image's rather than
# the operator's, so there is no document key to consult and no second copy.
SECRETS_PATH=
PEERS_PATH=
if [ -n "$SECRETS_DIR" ]; then
    if [ ! -d "$SECRETS_DIR" ]; then
        echo "$0: --secrets $SECRETS_DIR is not a directory" >&2
        exit 1
    fi
    SECRETS_PATH=$(crate_path "control plane identity" \
        "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_IDENTITY_DIR) || exit 1
    PEERS_PATH=$(crate_path "control plane pin file" \
        "$REPO/cmd/rivet-sysd/src/definitions.rs" MCPD_PEERS_PATH) || exit 1

    # The pin file lives INSIDE the identity directory today, and this script
    # places one directory. If that ever stops being true the placement would
    # silently drop the pin file somewhere nothing reads, and `rivet-mcpd` would
    # exit 2 naming a file the console showed being written.
    case $PEERS_PATH in
        "$SECRETS_PATH"/*) ;;
        *)  echo "$0: cmd/rivet-sysd's MCPD_PEERS_PATH ($PEERS_PATH) is not inside its" >&2
            echo "    MCPD_IDENTITY_DIR ($SECRETS_PATH), and this option places ONE" >&2
            echo "    directory. Refusing rather than putting the pin file nowhere." >&2
            exit 1 ;;
    esac

    # **All three, checked here.** `rivet-mcpd` reads the identity and the pins
    # before it binds a socket, so a directory missing one of them is a daemon
    # that exits 2 at start-up — and the run then looks like a guest that never
    # came up rather than like a harness that was handed half an identity.
    for _needed in device.seed device.crt "${PEERS_PATH##*/}"; do
        if [ ! -f "$SECRETS_DIR/$_needed" ]; then
            echo "$0: --secrets $SECRETS_DIR has no $_needed." >&2
            echo "    rivet-mcpd needs device.seed, device.crt and ${PEERS_PATH##*/}, and it" >&2
            echo "    reads all three BEFORE it binds — so a partial one exits 2 and" >&2
            echo "    the console shows a control plane that never listened." >&2
            echo "    \`cargo rivet identity new --directory $SECRETS_DIR\` writes the" >&2
            echo "    first two; the third is one hex public key per line." >&2
            exit 1
        fi
    done
fi

GDBSERVER_PATH=
if [ -n "$GDBSERVER_BIN" ]; then
    require_static_aarch64 "gdbserver binary" "$GDBSERVER_BIN"

    # cmd/rivet-image's constant, and it is the ONLY copy — verified two ways
    # (AGENTS.md §7): `grep -rn 'system/bin/gdbserver' --include=*.rs .` and
    # `git grep GDBSERVER_PATH`, which between them find this line and the
    # image crate's declaration and nothing else. Nothing in rivet-core knows
    # about a debugger, because nothing at runtime looks for one: ADR-005 §7.3
    # has an operator start it, so there is no probe and no service definition
    # for a second copy to be needed by.
    GDBSERVER_PATH=$(crate_path "gdbserver" \
        "$REPO/cmd/rivet-image/src/system.rs" GDBSERVER_PATH) || exit 1
fi

STRACE_PATH=
if [ -n "$STRACE_BIN" ]; then
    require_static_aarch64 "strace binary" "$STRACE_BIN"

    STRACE_PATH=$(crate_path "strace" \
        "$REPO/cmd/rivet-image/src/system.rs" STRACE_PATH) || exit 1
fi

# **The fixture's own constant, and it is the ONLY copy.** Every other path here
# is read out of the crate that LOOKS for the binary; this one is read out of
# the crate that IS the binary, because nothing on a Rivet looks for it — it is
# reached by `rdinit=` on the kernel command line, which this script composes.
# Read rather than written for the same reason all the rest are: a harness that
# packs a file where nothing execs it fails as a machine that never comes up,
# and the symptom names no file.
VHCI_PATH=
if [ -n "$VHCI_BIN" ]; then
    require_static_aarch64 "vhci fixture" "$VHCI_BIN"

    VHCI_PATH=$(crate_path "vhci fixture" \
        "$REPO/tools/qemu/vhci-fixture/src/main.rs" FIXTURE_PATH) || exit 1
fi

DOCUMENT_PATH=
if [ -n "$DOCUMENT_FILE" ]; then
    if [ ! -f "$DOCUMENT_FILE" ]; then
        echo "$0: no such document: $DOCUMENT_FILE" >&2
        exit 1
    fi
    # Composed from the two crates that own its halves, rather than written
    # here: rivet-core owns the directory (ADR-008 §3.1) and rivet-store
    # owns the file name for each ADR-007 §3.4 layer. cmd/rivet-sysd and
    # cmd/rivet-netd both reach it through ConfigStore, so a copy here would
    # be a third spelling of the same fact (ADR-011).
    CONFIG_DIR=$(crate_path "configuration directory" \
        "$REPO/rivet-core/src/state.rs" CONFIG_DIR) || exit 1
    MACHINE_FILE=$(sed -n \
        's/^pub const MACHINE_LAYER_FILE: *&str *= *"\([^"]*\)".*/\1/p' \
        "$REPO/rivet-store/src/store.rs" | head -1)
    if [ -z "$MACHINE_FILE" ]; then
        echo "$0: no MACHINE_LAYER_FILE in rivet-store/src/store.rs; refusing to guess" >&2
        exit 1
    fi
    DOCUMENT_PATH="$CONFIG_DIR/$MACHINE_FILE"
fi

DEPLOY_PATH=
if [ -n "$DEPLOY_BIN" ]; then
    require_static_aarch64 "installer binary" "$DEPLOY_BIN"

    # rivet-core's constant rather than the image crate's, and the difference
    # from NETD_PATH is the point: the installer's path is one of the four facts
    # rivet_core::deploy exists to hold, because four crates have to agree
    # about it and three of them are binaries that cannot `use` each other.
    DEPLOY_PATH=$(crate_path "installer"         "$REPO/rivet-core/src/deploy.rs" PROGRAM_PATH) || exit 1
fi

if [ -n "$DEPLOY_REQUEST" ] && [ "$DEPLOY_RAW_SET" = yes ]; then
    echo "$0: --deploy-request and --deploy-raw both write the same file, and a run" >&2
    echo "    that silently used one of them would prove whichever it happened to pick" >&2
    exit 2
fi

REQUEST_PATH=
REQUEST_LINE=
if [ "$DEPLOY_RAW_SET" = yes ]; then
    REQUEST_PATH=$(crate_path "install request"         "$REPO/rivet-core/src/deploy.rs" REQUEST_PATH) || exit 1
    REQUEST_LINE=$DEPLOY_RAW
fi
if [ -n "$DEPLOY_REQUEST" ]; then
    # Read out of rivet-core rather than spelled here, for DOCUMENT_PATH's
    # reason: cmd/rivet writes this path and cmd/rivet-sysd reads it,
    # and a third spelling in this file is exactly what ADR-011 is about.
    REQUEST_PATH=$(crate_path "install request"         "$REPO/rivet-core/src/deploy.rs" REQUEST_PATH) || exit 1

    # ...and so is the FORMAT, which grew a principal on 2026-08-24. Every piece
    # of the line below is extracted from rivet_core::deploy rather than typed
    # here: this file is a third party to a format whose one writer is
    # cmd/rivet and whose one reader is cmd/rivet-sysd, and a literal
    # `from=` here would be a copy that keeps working while the real one changes.
    DEPLOY_FIELD=$(crate_token "request's principal field"                                     "$REPO/rivet-core/src/deploy.rs" FROM_FIELD) || exit 1
    DEPLOY_MARKER=$(crate_token "request's payload marker"                                     "$REPO/rivet-core/src/deploy.rs" PAYLOAD_MARKER) || exit 1
    # **Only when --deploy-key was given**, and that is a repair rather than a
    # tidy. `KEY_FIELD` left rivet_core::deploy on 2026-09-06 with SSH
    # (`adde3717`), and this extraction ran unconditionally — so from that day
    # every `--deploy-request` run refused with "no KEY_FIELD found" before
    # QEMU started, whether or not a key was named. Found 2026-09-12, by being
    # the first person since to use the flag.
    DEPLOY_KEY_FIELD=
    if [ -n "$DEPLOY_KEY" ]; then
        DEPLOY_KEY_FIELD=$(crate_token "request's key field"                                     "$REPO/rivet-core/src/deploy.rs" KEY_FIELD) || exit 1
    fi
    if [ -z "$DEPLOY_FROM" ]; then
        # **The honest default.** This request is written by a shell script on a
        # development host, not by a control-plane session, and the token rivet-core
        # spells for that says exactly that and nothing more.
        DEPLOY_FROM=$(crate_token "request's no-session token"                                         "$REPO/rivet-core/src/deploy.rs" NO_SESSION_TOKEN) || exit 1
    fi
    REQUEST_LINE="$DEPLOY_FIELD$DEPLOY_FROM"
    if [ -n "$DEPLOY_KEY" ]; then
        REQUEST_LINE="$REQUEST_LINE $DEPLOY_KEY_FIELD$DEPLOY_KEY"
    fi
    REQUEST_LINE="$REQUEST_LINE $DEPLOY_MARKER $DEPLOY_REQUEST"
fi

KERNEL=${RIVET_KERNEL:-$HERE/kernel8.img}
if [ ! -f "$KERNEL" ]; then
    echo "$0: no kernel at $KERNEL" >&2
    echo "    run ./fetch-kernel.sh <user@host> first, or set RIVET_KERNEL." >&2
    exit 1
fi

# Build the network arguments, and refuse a kernel that provably cannot use
# them. The point of this check is that the alternative failure is silent: QEMU
# happily creates a device nothing probes, PID 1 comes up, the console looks
# entirely normal, and the only symptom is an interface that is not there. A
# session spent debugging Rivet's netlink code against a kernel with no
# driver is exactly what this costs if it is not caught here.
NET_ARGS=
if [ "$NET" = yes ]; then
    kernel_virtio_net "$KERNEL" && found=0 || found=$?
    case $found in
        1)
            echo "$0: $KERNEL contains no virtio-net driver, so --net would" >&2
            echo "    attach a device the guest cannot see. Refusing rather than" >&2
            echo "    producing a console that looks like a working boot with a" >&2
            echo "    broken Rivet in it." >&2
            echo >&2
            echo "    The stock Raspberry Pi kernel is such a kernel — verified" >&2
            echo "    2026-08-23 against virtio-mmio, virtio-pci and USB-ethernet," >&2
            echo "    none of which bound anything. Networking in this loop needs" >&2
            echo "    ADR-024's own kernel:" >&2
            echo >&2
            echo "        tools/buildroot/build.sh --fetch linux" >&2
            echo "        RIVET_KERNEL=<output>/images/Image $0 ..." >&2
            exit 1 ;;
        2)
            echo "note: could not read $KERNEL in a form this script can search," >&2
            echo "      so whether it has a virtio-net driver is UNKNOWN. Continuing." >&2
            echo "      If no interface appears, suspect the kernel before the code." >&2 ;;
    esac

    # `-netdev user` is QEMU's SLIRP stack: no root, no bridge, no host
    # configuration. It gives the guest 10.0.2.15/24 via DHCP with a gateway at
    # 10.0.2.2 — which means udhcpc has something to talk to, and that is most
    # of what Stage 1 wants to exercise.
    NETDEV="user,id=net0"
    # Each --forward-port, in the order given. Already validated above and each
    # element already carries its leading comma.
    NETDEV="$NETDEV$EXTRA_FORWARDS"

    # virtio-mmio by default. `-M virt` gives virtio-mmio slots unconditionally,
    # whereas the PCI form additionally needs the generic host bridge driver
    # (CONFIG_PCI_HOST_GENERIC) — one more thing that can be missing, for no gain
    # at this stage. --net-pci exists because the board's own NIC is not on an
    # mmio bus either, so being able to run the same code over a PCI-attached
    # NIC is worth having when something behaves differently on hardware.
    if [ "$NET_TRANSPORT" = pci ]; then
        NETDEVICE="virtio-net-pci,netdev=net0"
    else
        NETDEVICE="virtio-net-device,netdev=net0"
    fi
    NET_ARGS="-netdev $NETDEV -device $NETDEVICE"
fi

# Build the disk arguments, and check the image the way every other input to
# this script is checked: refuse rather than guess (`AGENTS.md` §5).
#
# Each of the three refusals below is a failure that would otherwise be read as
# a Rivet defect. A missing file makes QEMU exit before the kernel starts, which
# looks like the harness crashing. A block device given here is almost certainly
# a slip, and it is the one slip that can cost the development host its own disk
# — `AGENTS.md` §10.4 makes naming the device the guard, so this option will not
# take one at all. And an image with no partition table produces a guest whose
# `/dev/vda` has no partitions on it, so PID 1 reports the same ENOENT it
# reports with no disk attached and the run looks like a Rivet failure.
DISK_ARGS=
if [ -n "$CARD_IMAGE" ]; then
    if [ ! -e "$CARD_IMAGE" ]; then
        echo "$0: no such card image: $CARD_IMAGE" >&2
        echo "    tools/card/build.sh writes one into out/." >&2
        exit 1
    fi
    if [ ! -f "$CARD_IMAGE" ]; then
        echo "$0: --card $CARD_IMAGE is not a regular file." >&2
        echo "    This option takes an IMAGE FILE, and will not attach a block" >&2
        echo "    device: a guest booting onto the development host's own disk" >&2
        echo "    is the accident AGENTS.md §10.4 exists to prevent." >&2
        exit 1
    fi
    # The GPT header signature, at the start of LBA 1 — offset 512 on the
    # 512-byte sectors cmd/rivet-image writes. `od` rather than `dd` because
    # dd reports to stderr and its output would need the same decoding anyway.
    # The signature is `EFI PART`; od -c renders the space as its own field, so
    # stripping whitespace leaves EFIPART.
    _sig=$(od -A n -c -j 512 -N 8 "$CARD_IMAGE" 2>/dev/null | tr -d ' \t\n')
    if [ "$_sig" != "EFIPART" ]; then
        echo "$0: --card $CARD_IMAGE has no GPT header at LBA 1." >&2
        echo "    Read '$_sig' where the 'EFI PART' signature should be." >&2
        echo "    A guest given this would see /dev/vda with nothing on it, and" >&2
        echo "    PID 1 would report the same ENOENT it reports with no disk at" >&2
        echo "    all — so the run would look like a Rivet failure and would be" >&2
        echo "    a statement about this file." >&2
        exit 1
    fi

    # `if=none` and an explicit -device rather than the shorthand, because the
    # shorthand picks a bus and `-M virt` has no IDE or SCSI controller to pick.
    # virtio-mmio for the reason --net defaults to it: `-M virt` provides those
    # slots unconditionally, whereas the PCI form additionally needs the generic
    # host-bridge driver. ADR-024's kernel has both; one fewer thing that can be
    # missing is still worth having.
    #
    # `snapshot=on` unless --card-rw: QEMU allocates a temporary overlay and
    # every guest write goes there, so the named artifact is read-only in fact
    # rather than by intention.
    _card_media="file=$CARD_IMAGE,format=raw,if=none,id=card0"
    if [ "$CARD_RW" != yes ]; then
        _card_media="$_card_media,snapshot=on"
    fi
    DISK_ARGS="-drive $_card_media -device virtio-blk-device,drive=card0"
elif [ "$CARD_RW" = yes ]; then
    echo "$0: --card-rw without --card would do nothing: there is no image for" >&2
    echo "    it to write to. Refusing rather than accepting a flag that is" >&2
    echo "    silently inert — the run would look like a persistence test and" >&2
    echo "    would be an ordinary tmpfs boot." >&2
    exit 2
fi

# --card HIDES --document and --state, so the combination is refused.
#
# Both options write into the initramfs at a path under /state, and both were
# written for a world in which nothing was ever mounted there. PID 1 now mounts
# the card over that directory, so the files are still in the initramfs, still
# at the right paths, and unreachable to everything that looks after the mount.
#
# **This was found by an experiment that ran green and settled nothing.** Two
# boots with a document declaring several tables produced a
# /state with no `config` directory in it at all — read back off the card
# afterwards with debugfs — and the run had looked exactly like a run that
# worked. Refusing is the whole of AGENTS.md §5: an option that is silently
# ignored is worse than one that is absent, because it makes a report of what
# was tested untrue.
if [ -n "$CARD_IMAGE" ] && [ -n "$DOCUMENT_FILE" ]; then
    echo "$0: --document with --card would be IGNORED, so it is refused." >&2
    echo >&2
    echo "    --document packs the file into the INITRAMFS at the stored" >&2
    echo "    document's path, which is under /state. PID 1 then mounts the" >&2
    echo "    card over /state, and everything that reads the document after" >&2
    echo "    that — the supervisor, every service — looks at the card and" >&2
    echo "    finds nothing. The boot would run its no-configuration path" >&2
    echo "    while the console showed a document being packed." >&2
    echo >&2
    echo "    A machine with a card gets its document the way a board does:" >&2
    echo "    on /state, put there before the boot. Nothing in this harness" >&2
    echo "    can do that yet — PID 1's own provisioning path reads" >&2
    echo "    /proc/device-tree/chosen/bootloader/partition, which -M virt" >&2
    echo "    does not have, so a card built with RIVET_DOCUMENT does not" >&2
    echo "    self-provision here either. See PLAN.md §62.3." >&2
    exit 2
fi
if [ -n "$CARD_IMAGE" ] && [ -n "$SECRETS_DIR" ]; then
    echo "$0: --secrets with --card would be IGNORED, so it is refused." >&2
    echo >&2
    echo "    --secrets places the identity in the INITRAMFS under /state, and" >&2
    echo "    PID 1 mounts the card over it. rivet-mcpd would then read the" >&2
    echo "    CARD's /state/secrets, find nothing, and exit 2 — while this" >&2
    echo "    console showed an identity being placed." >&2
    echo >&2
    echo "    A card gets its identity the way a board does: cmd/rivet-image" >&2
    echo "    takes RIVET_IDENTITY and writes it onto p5 (ADR-025 §3), and" >&2
    echo "    tools/qemu/seed-card.sh is the other route onto an existing one." >&2
    exit 2
fi
if [ -n "$CARD_IMAGE" ] && [ -n "$STATE_DIR" ]; then
    echo "$0: --state with --card would be IGNORED, so it is refused." >&2
    echo >&2
    echo "    --state seeds the INITRAMFS /state, and PID 1 mounts the card" >&2
    echo "    over it. The seeded files survive in the initramfs and nothing" >&2
    echo "    can reach them." >&2
    echo >&2
    echo "    A card is a better answer to what --state was for in any case:" >&2
    echo "    --state existed to imitate a machine that has RUN BEFORE, and" >&2
    echo "    --card --card-rw produces one for real — boot it twice." >&2
    exit 2
fi

# This script ends in `exec`, which replaces the shell — so an EXIT trap would
# never fire, and the initramfs staged here would be left behind by every run.
# It was: 52 abandoned images and 76 MB of /tmp when this was noticed. There is
# no later moment at which this script still exists to tidy up after itself, so
# instead each run sweeps what earlier runs left. The age threshold is what
# makes that safe while another QEMU is still running off its own image.
WORKDIR=${TMPDIR:-/tmp}/rivet-qemu
mkdir -p "$WORKDIR"
find "$WORKDIR" -mindepth 1 -maxdepth 1 -mmin +60 -exec rm -rf {} + 2>/dev/null || true

ROOTFS=$(mktemp -d "$WORKDIR/run.XXXXXXXX")
# **`/` must be searchable by everything, and `mktemp -d` makes it 0700.**
#
# Found on 2026-08-24 by running, not by reading (`AGENTS.md` §6). The first
# boot with an application that runs under a uid of its own (`ADR-015` §5.1)
# refused with EACCES on a binary that was present and mode 0755, because
# `cpio` faithfully preserved this directory's 0700 and no unprivileged process
# on the guest could traverse `/` at all.
#
# It is a defect in the HARNESS and not in what it tests: `cmd/rivet-image`'s
# `system::DIR_MODE` is 0755, so no card this repository builds has a `/` like
# that. A harness modelling a root directory the builder cannot produce would
# have made every unprivileged process on the guest look broken.
chmod 755 "$ROOTFS"
# Fires only on the paths that exit before the exec below.
trap 'rm -rf "$ROOTFS" "$ROOTFS.cpio.gz"' EXIT
mkdir -p "$ROOTFS/dev"
pack "$INIT_BIN" "$INIT_PATH"

if [ -n "$SUPERVISOR_BIN" ]; then
    # The directories the supervisor needs at runtime are not created here.
    # /run is a tmpfs PID 1 mounts itself, and the notify sockets under
    # /run/rivet/notify are created by the supervisor's own open_notify()
    # with create_dir_all. Making them here would be making a claim about the
    # image that image assembly has to make. (This used to read "image
    # assembly, which does not exist yet"; cmd/rivet-image exists — corrected
    # 2026-08-23 rather than left standing, AGENTS.md §7.)
    pack "$SUPERVISOR_BIN" "$SUPERVISOR_PATH"
fi

# The CLI is packed to be *found* and nothing execs it — the projection's probe
# is a file_exists, so a binary sitting there is exactly what it reads. No
# console line below should be read as though the CLI had run.
#
# ~~rivet-netd is packed so that the change which starts it can be watched
# here.~~ **That change landed**: the supervisor has a definition for it and
# starts it (ADR-017 §4.3). Corrected 2026-08-23 rather than left standing.
if [ -n "$CLI_BIN" ]; then
    pack "$CLI_BIN" "$CLI_PATH"
fi
if [ -n "$NUSHELL_BIN" ]; then
    pack "$NUSHELL_BIN" "$NUSHELL_PATH"
fi
if [ -n "$NETD_BIN" ]; then
    pack "$NETD_BIN" "$NETD_PATH"
fi
if [ -n "$TIMED_BIN" ]; then
    pack "$TIMED_BIN" "$TIMED_PATH"
fi
if [ -n "$BLUETOOTHD_BIN" ]; then
    pack "$BLUETOOTHD_BIN" "$BLUETOOTHD_PATH"
fi
if [ -n "$PROXYD_BIN" ]; then
    pack "$PROXYD_BIN" "$PROXYD_PATH"
fi
if [ -n "$MCPD_BIN" ]; then
    pack "$MCPD_BIN" "$MCPD_PATH"
fi
if [ -n "$DEPLOY_BIN" ]; then
    pack "$DEPLOY_BIN" "$DEPLOY_PATH"
fi
if [ -n "$UDHCPC_BIN" ]; then
    pack "$UDHCPC_BIN" "$UDHCPC_PATH"
fi
if [ -n "$DHCP_HOOK_BIN" ]; then
    pack "$DHCP_HOOK_BIN" "$DHCP_HOOK_PATH"
fi
if [ -n "$GIT_BIN" ]; then
    pack "$GIT_BIN" "$GIT_PATH"
fi
if [ -n "$HTTP_BACKEND_BIN" ]; then
    pack "$HTTP_BACKEND_BIN" "$HTTP_BACKEND_PATH"
fi
if [ -n "$FONT_FILE" ]; then
    # `pack` chmods +x, which is wrong for a data file and harmless; the mode
    # is corrected so that the row in `ADR-042` §6's proof is a plain readable
    # file and not something that could be execed.
    pack "$FONT_FILE" "$FONT_PATH"
    chmod 644 "$ROOTFS$FONT_PATH"
fi
# The debugging mechanisms — ADR-005 §7. Packed to be *present*, exactly as the
# CLI is: nothing on the machine execs either, and no console line below should
# be read as though one had run.
if [ -n "$GDBSERVER_BIN" ]; then
    pack "$GDBSERVER_BIN" "$GDBSERVER_PATH"
fi
if [ -n "$STRACE_BIN" ]; then
    pack "$STRACE_BIN" "$STRACE_PATH"
fi
# The one binary here that is neither merely packed nor started by the
# supervisor: the kernel execs it, and it execs pid 1. See --vhci in the header.
if [ -n "$VHCI_BIN" ]; then
    pack "$VHCI_BIN" "$VHCI_PATH"
fi

# The /state MOUNT POINT, and only the mount point.
#
# This is the one directory this script creates that a running Rivet expects
# to find, and it is created for the reason the comment above the supervisor
# gives rather than against it: /state's mount point is part of the IMAGE
# (cmd/rivet-image's system::STATE_MOUNT_POINT, on the read-only erofs), so
# making it here is imitating image assembly, exactly as /system/bin is.
#
# **What it is NOT**: an ext4 on p5. QEMU -M virt has no /dev/mmcblk0, so PID 1's
# mount refuses with ENOENT and degrades (ADR-017 §3.1.7),
# and everything that then writes to /state is writing to the INITRAMFS tmpfs.
# That is enough to exercise a service precondition — mkdir with modes, a
# file generated once, the gate on the second pass — and it settles nothing
# about ext4, about the card, or about anything surviving a reboot. ADR-026 §4's
# right-hand column owns all three.
# **The trust store**, so an installed application can be verified rather than
# refused. PLAN.md §104.27.
if [ -n "${TRUST_FILE:-}" ]; then
    [ -f "$TRUST_FILE" ] || {
        echo "$0: --trust $TRUST_FILE is not a file" >&2
        exit 1
    }
    _trust=$(sed -n 's/^pub const PATH: *&str *= *"\([^"]*\)".*/\1/p' \
        "$REPO/rivet-core/src/package/trust_store.rs" | head -1)
    [ -n "$_trust" ] || {
        echo "$0: cannot read the trust store path out of rivet-core" >&2
        exit 1
    }
    pack "$TRUST_FILE" "$_trust"
    echo "$0: placed a trust store at $_trust ($(wc -c < "$TRUST_FILE") bytes)"
fi

# **The architecture record, on every run** — rivet_core::package::arch::PATH,
# PLAN-Feature-Set-2026-09-12 §3.3g. A machine whose image records no
# architecture admits no artifact at its store's door and installs none, so a
# guest without this file could prove nothing about installing anything.
#
# **Always `aarch64`, and that is a fact about this harness rather than a
# default**: it boots qemu-system-aarch64 -M virt and packs binaries built for
# aarch64-unknown-linux-musl, so the token for what this guest executes is
# fixed by what this script is. The path is read out of rivet-core, as the
# trust store's is; the content is one ADR-027 §3.1 token and a newline, which
# is the whole of the format `rivet_core::package::arch::read` accepts.
_arch_path=$(sed -n 's/^pub const PATH: *&str *= *"\([^"]*\)".*/\1/p' \
    "$REPO/rivet-core/src/package/arch.rs" | head -1)
[ -n "$_arch_path" ] || {
    echo "$0: cannot read the architecture record's path out of rivet-core" >&2
    exit 1
}
mkdir -p "$ROOTFS$(dirname "$_arch_path")"
printf 'aarch64\n' >"$ROOTFS$_arch_path"
chmod 0444 "$ROOTFS$_arch_path"
echo "$0: placed the architecture record at $_arch_path (aarch64)"

# **The loader and the shared libraries** — PLAN.md §104.27.
#
# Placed before /state so that a dynamic application unpacked there has
# everything it needs by the time PID 1 runs.
if [ -n "${LIBRARIES_DIR:-}" ]; then
    [ -d "$LIBRARIES_DIR" ] || {
        echo "$0: --libraries $LIBRARIES_DIR is not a directory" >&2
        exit 1
    }
    # The loader IS the C library on musl, and it is read from the cross
    # sysroot for the same reason tools/card/build.sh reads it there: it is the
    # file the toolchain writes into every dynamic binary's PT_INTERP, and a
    # different copy would be a different libc.
    _loader=$(sed -n 's/^pub const LOADER_PATH: *&str *= *"\([^"]*\)".*/\1/p' \
        "$REPO/rivet-core/src/package/library.rs" | head -1)
    [ -n "$_loader" ] || {
        echo "$0: cannot read LOADER_PATH out of rivet-core; refusing to guess" >&2
        echo "    where a dynamic binary's interpreter belongs." >&2
        exit 1
    }
    # **Derived from the kernel's own path**, because this harness is already
    # pointed at a Buildroot tree by `RIVET_KERNEL` and a second variable
    # naming the same tree is a second thing that can disagree (`ADR-011`).
    _sysroot=${RIVET_SYSROOT:-$(dirname "$(dirname "$KERNEL")")/host/aarch64-buildroot-linux-musl/sysroot}
    _libc="$_sysroot/lib/libc.so"
    [ -f "$_libc" ] || {
        echo "$0: no musl loader at $_libc" >&2
        echo "    Derived from RIVET_KERNEL ($KERNEL). Set RIVET_SYSROOT to the" >&2
        echo "    cross sysroot if the Buildroot tree is laid out differently." >&2
        exit 1
    }
    pack "$_libc" "$_loader"
    _packed=0
    for _so in "$LIBRARIES_DIR"/*.so*; do
        [ -e "$_so" ] || continue
        pack "$_so" "/lib/$(basename "$_so")"
        _packed=$((_packed + 1))
    done
    echo "$0: packed the musl loader at $_loader and $_packed shared librar\
$([ "$_packed" = 1 ] && echo y || echo ies) into /lib"
fi

mkdir -p "$ROOTFS/state"

# ...and what is IN it, if a run wants a machine that has been up before.
#
# `cp -a` so modes and ownership survive the copy — which is the whole point of
# the option: a test that seeded a directory and then had `cp` normalise its
# mode would assert nothing.
if [ -n "$STATE_DIR" ]; then
    if [ ! -d "$STATE_DIR" ]; then
        echo "$0: --state $STATE_DIR is not a directory" >&2
        exit 1
    fi
    cp -a "$STATE_DIR"/. "$ROOTFS/state/" || {
        echo "$0: could not seed /state from $STATE_DIR" >&2
        exit 1
    }
fi

# This machine's TLS identity and the keys allowed to connect to it, if a
# directory was given. `cp -a` for --state's reason: `device.seed` is minted
# mode 0600 and a copy that normalised it would be a harness quietly relaxing a
# permission on a private key.
#
# **On a card this comes from somewhere else entirely** — `cmd/rivet-image`
# takes `RIVET_IDENTITY` and writes it onto p5 at provisioning time (`ADR-025`
# §3, `ADR-019` §2: credentials never traverse a network to enroll). So a run
# made this way settles nothing about enrollment, exactly as --document settles
# nothing about provisioning. What it makes reachable is everything downstream
# of a machine that HAS an identity.
if [ -n "$SECRETS_DIR" ]; then
    mkdir -p "$ROOTFS$SECRETS_PATH"
    cp -a "$SECRETS_DIR"/. "$ROOTFS$SECRETS_PATH/" || {
        echo "$0: could not place the control-plane identity from $SECRETS_DIR" >&2
        exit 1
    }
    chmod 700 "$ROOTFS$SECRETS_PATH"
fi

# The stored document, if one was given. This is the ONE thing in the initramfs
# that a card gets from somewhere else entirely — PID 1 reads the provisioning
# file off the FAT boot partition and stores it (ADR-017 §3.1.9) — so a run made
# this way settles nothing at all about provisioning. What it makes reachable is
# everything downstream of a machine that HAS a document.
if [ -n "$DOCUMENT_FILE" ]; then
    mkdir -p "$ROOTFS$(dirname "$DOCUMENT_PATH")"
    cp "$DOCUMENT_FILE" "$ROOTFS$DOCUMENT_PATH"
fi

# The install request, if one was named. The maintainer's 2026-08-24 ruling: this is what
# `rivet apply install <path> confirm` leaves on /state, and the supervisor
# takes it up on its next convergence pass.
#
# **It is not an install** — see --deploy-request in the usage above. Nothing
# here verifies a payload, nothing signs anything, and cmd/rivet-deploy is not
# packed by this script at all, so the convergence that follows will refuse to
# spawn a program that is not in the image. What that still settles is the whole
# reading half of the channel on a real kernel: the file is found, parsed,
# removed, and a service instance exists that did not exist when the supervisor
# started (ADR-017 §3.2.2) — and, since 2026-08-24, that the principal in the
# request reaches a record (ADR-028 §4.5).
#
# **The principal here is a claim this script writes into a file**, which is
# precisely what anything with root on /state can also do. A run proves the
# supervisor reads and records it; it proves nothing about anybody's authority,
# because nothing anywhere checks one.
if [ -n "$REQUEST_PATH" ]; then
    mkdir -p "$ROOTFS$(dirname "$REQUEST_PATH")"
    printf '%s
' "$REQUEST_LINE" > "$ROOTFS$REQUEST_PATH"
    chmod 600 "$ROOTFS$REQUEST_PATH"
fi

# The artifact the request names, as far as the SUPERVISOR is concerned.
#
# ADR-052 §7.2: a request names a digest and the supervisor resolves it under
# /state/artifacts/objects/<digest>/, which holds exactly one file. Without this
# a --deploy-request run would exercise the store's "this machine does not hold
# it" refusal on every boot — a real branch, and not the one the flag is for.
#
# **It is a placeholder and this is the third time this file says so.** Its
# contents are not the digest that names it and nothing on either side checks
# that they are: the transfer that would have hashed them is not in this path.
# What it makes reachable is the resolution — a readdir that finds one file and
# a path on an argument vector.
#
# **`tools/qemu/adr-052-proof.sh` is the other half**, and it does not use this
# flag: it boots a machine with a control plane and lets `cargo rivet deploy`
# put REAL signed artifacts in the store through the bulk endpoint, which is how
# a machine actually gets one.
if [ -n "$DEPLOY_REQUEST" ]; then
    ARTIFACTS_DIR=$(crate_path "artifact store"         "$REPO/rivet-core/src/state.rs" ARTIFACTS) || exit 1
    OBJECT_DIR="$ROOTFS$ARTIFACTS_DIR/objects/$DEPLOY_REQUEST"
    mkdir -p "$OBJECT_DIR"
    printf 'not a payload: see --deploy-request in %s\n' "$(basename "$0")" \
        > "$OBJECT_DIR/placeholder.rvt"
fi

# Installed applications, if any were named. ADR-015 §2 puts them under
# /state/app/<name>, §7 puts the live version behind a `current` link, and
# rivet_core::service composes the two paths the supervisor reads:
# /state/app/<name>/current/manifest.toml and .../bin/<name>.
#
# **This is not an install** — see --app in the usage above. It leaves behind
# what an installer would leave behind, so that ADR-017 §4's application-service
# path has something to read while ADR-029 §12 item 4's installer does not
# exist. Nothing here signs anything, and the supervisor says so on every boot.
#
# The paths are composed here rather than read out of rivet-core, unlike every
# binary above. That is a deliberate exception with a cost: APP_ROOT, the
# `current` link name and `bin` are three spellings this file now owns a second
# copy of. They are constants in one module, `rivet_core::service`, and the
# extraction device the other paths use (grep the crate for the literal) needs
# the value to be a `&str` constant on its own line — these are assembled by
# `format!` inside functions, so there is nothing to grep for. The tripwire is
# that a run with --app produces "no applications are installed" on the console
# if this drifts, which is loud rather than silent.
for _app in $APPS; do
    _name=${_app%%:*}
    _rest=${_app#*:}
    _bin=${_rest%%:*}
    _rest=${_rest#*:}
    _manifest=${_rest%%:*}
    # **An optional FOURTH field: the retained signed header** — added
    # 2026-09-05 for `ADR-042` §6. Absent, this is exactly what it always was,
    # and the console still says NOT signed. Present, it is what an installer
    # leaves at `rivet_core::package::install::HEADER_NAME`, without which
    # `cmd/rivet-sysd`'s verify refuses every launch — *"no signed header ...
    # so nothing can be verified"* — and the supervisor DEFINES the service and
    # never starts it, which is a machine no proof can be run on.
    case $_rest in
        *:*) _header=${_rest#*:} ;;
        *)   _header= ;;
    esac
    if [ -z "$_name" ] || [ -z "$_bin" ] || [ -z "$_manifest" ] || [ "$_bin" = "$_manifest" ]; then
        echo "$0: --app wants NAME:BINARY:MANIFEST[:HEADER], got '$_app'" >&2
        exit 2
    fi
    if [ -n "$_header" ] && [ ! -f "$_header" ]; then
        echo "$0: --app '$_name': no header at '$_header'" >&2
        exit 2
    fi
    if [ ! -f "$_bin" ]; then
        echo "$0: --app '$_name': no binary at '$_bin'" >&2
        exit 2
    fi
    if [ ! -f "$_manifest" ]; then
        echo "$0: --app '$_name': no manifest at '$_manifest'" >&2
        exit 2
    fi
    _dir=/state/app/$_name/current
    mkdir -p "$ROOTFS$_dir/bin"
    cp "$_manifest" "$ROOTFS$_dir/manifest.toml"
    cp "$_bin" "$ROOTFS$_dir/bin/$_name"
    chmod +x "$ROOTFS$_dir/bin/$_name"
    if [ -n "$_header" ]; then
        # The name is rivet_core::package::install::HEADER_NAME. Spelled here
        # for the reason the paragraph above the loop gives about APP_ROOT and
        # `current`: it is a constant in a module this shell cannot read, and
        # the tripwire is that the supervisor refuses by name if it drifts.
        cp "$_header" "$ROOTFS$_dir/package-header"
        chmod 444 "$ROOTFS$_dir/package-header"
        echo "run.sh: installed application '$_name' at $_dir, WITH a signed header" >&2
        echo "        (it will be verified against whatever --trust put on this machine)" >&2
    else
        echo "run.sh: installed application '$_name' at $_dir (NOT signed, NOT verified)" >&2
    fi
done

# Without /dev/console the kernel gives init no stdio, and a working boot is
# indistinguishable from a hang. ADR-026 §3.
if [ "$(id -u)" = 0 ]; then
    mknod "$ROOTFS/dev/console" c 5 1
    ( cd "$ROOTFS" && find . | cpio -o -H newc 2>/dev/null ) | gzip > "$ROOTFS.cpio.gz"
elif command -v fakeroot >/dev/null 2>&1; then
    fakeroot sh -c "mknod '$ROOTFS/dev/console' c 5 1
        cd '$ROOTFS' && find . | cpio -o -H newc 2>/dev/null" | gzip > "$ROOTFS.cpio.gz"
else
    echo "$0: need root or fakeroot to create /dev/console in the initramfs" >&2
    exit 1
fi

# rdinit= not init= — the binary is in the initramfs, so no disk, no partitions,
# no bootloader. Anything after `--` on the kernel command line is handed to
# init as its argv, which is the only way arguments reach a process the kernel
# starts.
RDINIT=$INIT_PATH
if [ "$SUPERVISOR_AS_INIT" = yes ]; then
    RDINIT=$SUPERVISOR_PATH
fi
if [ -n "$VHCI_BIN" ]; then
    # **The fixture is what the kernel execs, and pid 1 is what the fixture
    # execs.** Its argv is the plan, then the path it is to hand this process
    # to, then whatever that program's own arguments were — so the machine that
    # comes up is the one that would have come up, plus one process.
    #
    # A plan with a space in it would put a fragment of itself where the init
    # path belongs; `--vhci-plan` refuses one at the shell, above, for that
    # reason. The default is `count=0`, spelled here rather than left empty so
    # that the argv the fixture sees is always three-or-more words and a reader
    # of `/proc/cmdline` can see which of the two is the plan.
    APPEND="earlycon console=ttyAMA0 rdinit=$VHCI_PATH -- ${VHCI_PLAN:-count=0} $RDINIT"
    if [ -n "$SUPERVISOR_ARGS" ]; then
        APPEND="$APPEND $SUPERVISOR_ARGS"
    fi
else
    APPEND="earlycon console=ttyAMA0 rdinit=$RDINIT"
    if [ -n "$SUPERVISOR_ARGS" ]; then
        APPEND="$APPEND -- $SUPERVISOR_ARGS"
    fi
fi

# The staging tree has served its purpose; only the archive is still needed.
rm -rf "$ROOTFS"
trap 'rm -f "$ROOTFS.cpio.gz"' EXIT

echo "kernel:     $KERNEL"
echo "init:       $INIT_BIN -> $INIT_PATH"
echo "            so /proc/1/comm reads the same name a board gives it, and"
echo "            cmd/rivet's interlock recognises this machine (ADR-026 4.4)."
if [ -n "$SUPERVISOR_BIN" ]; then
    echo "supervisor: $SUPERVISOR_BIN -> $SUPERVISOR_PATH"
else
    echo "supervisor: none packed — PID 1 will retry, back off and give up (its failure path)"
fi
if [ -n "$CLI_BIN" ]; then
    echo "cli:        $CLI_BIN -> $CLI_PATH"
    echo "            the /etc projection probes that path, so it should report"
    echo "            names_a_login_shell: true. Nothing execs the CLI."
else
    echo "cli:        none packed — the projection will find nothing at the path it"
    echo "            probes, /etc/shells will name no shell, and the run will say"
    echo "            \"permits_a_login\":false. That is a fact about this"
    echo "            initramfs, not a defect."
fi
if [ -n "$NUSHELL_BIN" ]; then
    echo "nushell:    $NUSHELL_BIN -> $NUSHELL_PATH"
    echo "            it is first in LOGIN_SHELL_PATHS, so root's shell should be nu"
    echo "            and /etc/shells should name two. Nothing execs it."
else
    echo "nushell:    none packed — the probe will fall through to the CLI, which is"
    echo "            the card the ruling that makes nu optional requires to keep working."
fi
if [ -n "$DEPLOY_BIN" ]; then
    echo "deploy:     $DEPLOY_BIN -> $DEPLOY_PATH"
    echo "            NOTHING STARTS IT unless --deploy-request also names a payload"
fi
if [ -n "$REQUEST_PATH" ]; then
    echo "request:    $REQUEST_LINE"
    echo "            -> $REQUEST_PATH"
    if [ "$DEPLOY_RAW_SET" = yes ]; then
        echo "            written VERBATIM by --deploy-raw; the supervisor is expected to refuse it"
    else
        echo "            the supervisor consumes it on its first convergence pass"
        echo "            the principal in it is a CLAIM this script wrote, never a checked one"
    fi
fi
if [ -n "$NETD_BIN" ]; then
    echo "netd:       $NETD_BIN -> $NETD_PATH"
fi
if [ -n "$UDHCPC_BIN" ]; then
    echo "udhcpc:     $UDHCPC_BIN -> $UDHCPC_PATH"
fi
if [ -n "$DHCP_HOOK_BIN" ]; then
    echo "dhcp hook:  $DHCP_HOOK_BIN -> $DHCP_HOOK_PATH"
    echo "            the supervisor has a definition for it and starts it"
fi
if [ -n "$TIMED_BIN" ]; then
    echo "timed:      $TIMED_BIN -> $TIMED_PATH"
    echo "            RUN. It emits ADR-022 §3 records rather than lines, so"
    echo "            the supervisor's capture forwards them as records —"
    echo "            §4's first path, and this is its only producer."
else
    echo "timed:      none packed — its service will REFUSE with ENOENT, which"
    echo "            is true of this initramfs and hides ADR-022 §4's first"
    echo "            capture path behind having nothing to capture."
fi
if [ -n "$MCPD_BIN" ]; then
    echo "mcpd:       $MCPD_BIN -> $MCPD_PATH"
    echo "            RUN. The supervisor's definition is enabled and starts it"
    echo "            with --document, --identity and --peers (ADR-035 phase 1)."
    if [ -z "$CLI_BIN" ]; then
        echo "            *** NO CLI PACKED: it hands each caller the stdin and"
        echo "                stdout of the CLI, so a connection will be accepted"
        echo "                and the spawn will fail. Pack --cli."
    fi
    if [ -z "$SECRETS_DIR" ]; then
        echo "            *** NO --secrets: it reads the identity and the pins"
        echo "                BEFORE it binds, so it will exit 2 and never listen."
        echo "                That is a machine with no identity on /state."
    fi
else
    echo "mcpd:       none packed — the supervisor's definition is enabled, so the"
    echo "            spawn REFUSES with ENOENT and it says so once, naming the"
    echo "            service. That is true of this initramfs, and it means THIS"
    echo "            RUN HAS NO ROUTE IN: nothing can ask this machine anything."
fi
if [ -n "$SECRETS_DIR" ]; then
    echo "secrets:    $SECRETS_DIR -> $SECRETS_PATH"
    echo "            this machine's device.seed and device.crt, and the pins in"
    echo "            $PEERS_PATH. Standing in for cmd/rivet-image writing them"
    echo "            onto p5 at provisioning (ADR-025 §3) — enrollment is NOT"
    echo "            tested here. They land on the INITRAMFS tmpfs."
fi
if [ -n "$STATE_DIR" ]; then
    echo "state:      SEEDED from $STATE_DIR"
    echo "            so this is a boot onto a machine that has run before, which"
    echo "            is the only way that case is reachable here."
fi
if [ -n "$FONT_FILE" ]; then
    echo "font:       $FONT_FILE -> $FONT_PATH"
    echo "            image content, imitated. NOTHING DRAWS HERE - it is there"
    echo "            so ADR-042 §6's probe can prove the grant reaches a file"
    echo "            that exists, rather than reporting ENOENT."
fi
if [ -n "$GIT_BIN" ]; then
    echo "git:        $GIT_BIN -> $GIT_PATH"
    echo "            RUN by two things: the supervisor, for git init --bare"
    echo "            converging [repos], and git-http-backend, which runs git"
    echo "            rather than reaching upload-pack by --exec-path. The repo"
    echo "            lands on the INITRAMFS tmpfs, not on an ext4 /state -"
    echo "            nothing here survives a reboot."
else
    echo "git:        none packed - [repos] will REFUSE at git init and a clone"
    echo "            over the control plane serves an EMPTY repository."
fi
if [ -n "$HTTP_BACKEND_BIN" ]; then
    echo "git-http:   $HTTP_BACKEND_BIN -> $HTTP_BACKEND_PATH"
    echo "            execed by rivet-mcpd for every git request on the control plane."
fi
if [ -n "$GDBSERVER_BIN" ]; then
    echo "gdbserver:  $GDBSERVER_BIN -> $GDBSERVER_PATH"
    echo "            PRESENT AND NOT RUN. Nothing on the machine starts it;"
    echo "            ADR-005 §7.3 has an operator do that themselves. A green"
    echo "            boot says the binary is there and nothing more."
fi
if [ -n "$STRACE_BIN" ]; then
    echo "strace:     $STRACE_BIN -> $STRACE_PATH"
    echo "            PRESENT AND NOT RUN, on the same terms."
fi
if [ -n "$BLUETOOTHD_BIN" ]; then
    echo "bluetoothd: $BLUETOOTHD_BIN -> $BLUETOOTHD_PATH"
    echo "            DEFINED AND DISABLED (ADR-041 §6.5). Only a --document that"
    echo "            sets [services.rivet-bluetoothd] enabled = true starts it."
fi
if [ -n "$PROXYD_BIN" ]; then
    echo "proxyd:     $PROXYD_BIN -> $PROXYD_PATH"
    echo "            DEFINED AND DISABLED (ADR-060). Only a --document that"
    echo "            sets [services.rivet-proxyd] enabled = true starts it."
fi
if [ -n "$VHCI_BIN" ]; then
    echo "vhci:       $VHCI_BIN -> $VHCI_PATH, plan '${VHCI_PLAN:-count=0}'"
    echo "            THE KERNEL EXECS THIS, NOT PID 1. It starts one child and"
    echo "            execs $RDINIT, so pid 1 is what it always was. The child"
    echo "            opens /dev/vhci and IS this machine's Bluetooth controller."
    echo "            It settles NOTHING about a radio (ADR-026 §4): every byte"
    echo "            it delivers was composed by the fixture."
fi
if [ -n "$DOCUMENT_FILE" ]; then
    echo "document:   $DOCUMENT_FILE -> $DOCUMENT_PATH"
    echo "            standing in for PID 1 seeding it off the boot partition,"
    echo "            which -M virt cannot do. Provisioning is NOT tested here."
else
    echo "document:   none packed — this machine has stated no desired state, so"
    echo "            rivet-netd exits 2 on every pass and there is no address."
fi
if [ -n "$CARD_IMAGE" ]; then
    echo "card:       $CARD_IMAGE"
    echo "            attached as virtio-blk. The guest sees /dev/vda and the"
    echo "            partitions of the GPT cmd/rivet-image wrote — NOT"
    echo "            /dev/mmcblk0, and with no 'p' before the number."
    if [ "$CARD_RW" = yes ]; then
        echo "            *** --card-rw: WRITES GO TO THE FILE and outlive this run."
    else
        echo "            Writes go to a discarded overlay and the file is left"
        echo "            alone, so nothing here can survive the run. --card-rw"
        echo "            is what changes that."
    fi
    echo "            ADR-026 §4: this is a file behind virtio-blk. It settles"
    echo "            the partition table and ext4; it settles NOTHING about"
    echo "            SD/MMC timing, card wear or the firmware."
else
    echo "state:      /state exists as a mount point only. QEMU has no /dev/mmcblk0,"
    echo "            so PID 1's mount refuses and everything written there goes to"
    echo "            the initramfs tmpfs. Nothing here settles ext4 or the card."
    echo "            --card <image> is how that changes (PLAN.md §61.2)."
fi
if [ "$NET" = yes ]; then
    echo "network:    virtio-net over $NET_TRANSPORT, user-mode (guest 10.0.2.15/24, gw 10.0.2.2)"
    if [ -z "$EXTRA_FORWARDS" ]; then
        echo "            NO --forward-port: SLIRP has no route from the host into"
        echo "            the guest, so nothing here is reachable from outside."
    fi
    if [ -n "$EXTRA_FORWARDS" ]; then
        echo "            extra forwards:$(echo "$EXTRA_FORWARDS" | tr ',' ' ')"
    fi
    echo "            ADR-026 §4: this is virtio, NOT bcmgenet. It tests Rivet's"
    echo "            netlink and DHCP code and settles nothing about the board's NIC."
else
    echo "network:    none"
fi
if [ "$SUPERVISOR_AS_INIT" = yes ]; then
    echo
    echo "*** rdinit=$RDINIT — the SUPERVISOR is pid 1 in this run. ***"
    echo "    rivet-initd is packed at /init and is NOT running. Nothing below"
    echo "    demonstrates PID 1 spawning, restarting or stopping a supervisor;"
    echo "    it demonstrates the supervisor itself, with arguments PID 1 has no"
    echo "    way to pass. Nothing mounts /proc, /sys or /run in this mode either,"
    echo "    so the supervisor's degraded branches are what you are watching."
fi
echo "exit qemu:  Ctrl-A x"
echo

# -cpu cortex-a72 is the Pi 4's core (ADR-002 §2).
#
# $NET_ARGS and $DISK_ARGS are deliberately unquoted: each is a small set of
# shell words this script built itself, and the alternative — an array — is not
# available in POSIX sh. Every value inside $NET_ARGS is either a literal or a
# digits-only port this script has already validated, so there is nothing there
# for word splitting to get wrong.
#
# $DISK_ARGS is the one that carries a path the caller supplied, and that is a
# stated edge rather than an oversight: a --card path containing a space or a
# comma would be split, or read by QEMU as the end of the option. Both fail
# loudly at start-up with the path in the message. A validation here would have
# to refuse paths QEMU's own `file=` escaping can express, which is a worse
# trade than a clear failure.
exec qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a72 \
    -m 2G \
    -kernel "$KERNEL" \
    -initrd "$ROOTFS.cpio.gz" \
    -append "$APPEND" \
    -nographic \
    -no-reboot \
    $NET_ARGS \
    $DISK_ARGS \
    "$@"
