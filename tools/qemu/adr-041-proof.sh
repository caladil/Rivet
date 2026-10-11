#!/bin/sh
# tools/qemu/adr-041-proof.sh — **an advertisement crosses the endpoint, under
# emulation.**
#
# `ADR-041` §8.4. Run it in the container:
#
#   tools/container/exec.sh sh tools/qemu/adr-041-proof.sh
#
# ## What it is for
#
# Bluetooth tier 1 was closed on hardware on 2026-09-05 — the radio attaches,
# `rivet-bluetoothd` scans, and two applications have read the maintainer's own
# sensors through the endpoint. What had never happened is any of that **under
# emulation**, and three of §8.4's open items were open because of it: the
# drop-and-count path had only ever been taken by a unit test over a
# `socketpair`, the refusal of a second consumer had only ever been taken by a
# unit test over a list of names, and no boot had ever come up scanning from
# its own document rather than from a reload on a running machine.
#
# `tools/qemu/vhci-fixture` is what makes those reachable: `CONFIG_BT_HCIVHCI`
# lets a process BE a controller, so a synthetic advertisement can be pushed in
# at the bottom of the stack and followed all the way out of the top.
#
# ## Four boots, and no one of them alone is the proof
#
# **Stage 1** — one consumer, a steady trickle of advertisements:
#
#   * the fixture creates a controller and the kernel registers it;
#   * `rivet-bluetoothd` finds it, takes its user channel, and runs the
#     five-command scan setup — which the fixture prints as it answers, so the
#     order in `cmd/rivet-bluetoothd/src/hci.rs` is checked against a real
#     kernel rather than against its own test; and
#   * an application that declares nothing but `connects` **prints
#     advertisements it was handed**, with the addresses, the name and the
#     signal strengths this fixture composed and nothing else knew.
#
# **Stage 1b** — the same application and a SECOND one declaring the same
# `connects`, on a boot of their own:
#
#   * the endpoint is not shared, so **both are refused and neither is
#     started** (`ADR-015` §5.3), and each refusal names both
#     claimants. Stage 1 is the control: the same binary, manifest and header
#     were granted the endpoint when nothing contested it.
#
# **Stage 2** — one consumer that takes the endpoint and never reads it, and
# twenty thousand advertisements:
#
#   * the daemon **drops, counts and reports**, and never blocks; and
#   * the count it reports is smaller than the number injected, which is the
#     within-boot control: writes were succeeding until the buffer filled, so
#     what is being watched is back-pressure and not a socket that never worked.
#
# **Stage 3** — the controller unplugged once and plugged back in:
#
#   * the daemon, holding no capability after its first bind, exits 4 when the
#     controller goes away; and
#   * the supervisor starts it again, it binds the new controller, gives up
#     every capability again and scans (`ADR-041` §6.6).
#
# Stage 1 also asserts that the daemon scans holding nothing: every capability
# set empty and `NoNewPrivs` set, in its own journal line.
#
# ## What it does NOT settle, stated plainly
#
# * **Nothing about a radio.** `ADR-026` §4's right-hand column. Whether the
#   CYW43455 attaches over serdev, takes its patchram and hears a neighbour's
#   sensor is the board's to say, and `ADR-041` §8.1–§8.3 is where it said it.
# * **Nothing about the air.** Every byte that crosses the endpoint here was
#   composed by `tools/qemu/vhci-fixture`. What is proven is the PATH.
# * **Nothing about rates.** A `write(2)` loop under TCG is not eighteen
#   advertisements a second from twenty-nine devices in a house, and the
#   `ble-backpressure` fixture is slow by refusing to read rather than by being
#   a real application that is busy.
# * **Nothing about power**, which §8.4's first item says needs an instrument.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TARGET=aarch64-unknown-linux-musl
# **`CARGO_TARGET_DIR` and not `$REPO/target`** — `tools/container/exec.sh`
# mounts a named volume there and exports it, so a script that composed the
# in-tree path would build correctly and then look where nothing put the
# binaries. Every fixture workspace below lands in the same volume.
OUT=${CARGO_TARGET_DIR:-$REPO/target}/$TARGET/release

STAGE=
say() { echo "adr-041-proof: $*"; }
fail() {
    echo >&2
    echo "adr-041-proof: FAIL${STAGE:+ [$STAGE]}: $*" >&2
    exit 1
}

# --- what the guest is made of ---------------------------------------------

export CC_aarch64_unknown_linux_musl="${CC_aarch64_unknown_linux_musl:-/br/host/bin/aarch64-linux-gcc}"

say "building pid 1, the supervisor and the daemon for $TARGET"
( cd "$REPO" && cargo build --release --target "$TARGET" \
    -p rivetos-initd -p rivetos-sysd -p rivetos-bluetoothd ) \
    || fail "the guest's binaries did not build"

say "building the virtual controller and the two consumers"
( cd "$HERE/vhci-fixture" && cargo build --release --target "$TARGET" ) \
    || fail "tools/qemu/vhci-fixture did not build"
# **The real probe**, not one written here. `ADR-041` §4.3 put `ble-listen` in
# the design record because it is what took §8.3's measurements on the board;
# using the same program under emulation is what makes the two runs comparable.
( cd "$REPO/examples/probes/ble-listen" && cargo build --release --target "$TARGET" ) \
    || fail "examples/probes/ble-listen did not build"
( cd "$HERE/ble-backpressure" && cargo build --release --target "$TARGET" ) \
    || fail "tools/qemu/ble-backpressure did not build"

for _needed in rivet-initd rivet-sysd rivet-bluetoothd rivet-vhci-fixture ble-listen ble-backpressure; do
    [ -x "$OUT/$_needed" ] || fail "no $_needed at $OUT/$_needed"
done

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rivet-adr041.XXXXXX") || fail "no temporary directory"
chmod 700 "$WORK"

# --- the two manifests, RENDERED rather than hand-maintained ----------------
#
# `docs/design-candidates/SDK-Adoption-Friction.md` §2.3: the only
# specification of this format is `Manifest::render`
# (`rivet_core::package::application`), and this fixture used to carry two
# committed `.manifest.toml` files as a hand-copy of what it emits. Rendered
# here, into `$WORK`, by the tool that owns the format — a HOST build, since
# printing a manifest reads only `Cargo.toml` and needs no target triple.
say "building cargo-rivet, to render manifests instead of hand-copying them"
( cd "$REPO" && cargo build --release -p cargo-rivet ) || fail "cargo-rivet did not build"
CARGO_RIVET=${CARGO_TARGET_DIR:-$REPO/target}/release/cargo-rivet
[ -x "$CARGO_RIVET" ] || fail "no cargo-rivet at $CARGO_RIVET"

"$CARGO_RIVET" package --print-manifest \
    --manifest-path "$REPO/examples/probes/ble-listen/Cargo.toml" \
    >"$WORK/ble-listen.manifest.toml" \
    || fail "could not render examples/probes/ble-listen's manifest"
"$CARGO_RIVET" package --print-manifest \
    --manifest-path "$HERE/ble-backpressure/Cargo.toml" \
    >"$WORK/ble-backpressure.manifest.toml" \
    || fail "could not render tools/qemu/ble-backpressure's manifest"

# --- signing, because the supervisor verifies before every launch -----------
#
# `ADR-015` §4.2, and `tools/qemu/adr-042-proof.sh` hit the same wall first:
# without a retained header and a trust store the supervisor DEFINES each
# service and refuses to start it, which is correct and is a machine on which
# nothing can be observed. The signer's key is a fixture and its own header
# says why it is not a credential.
say "signing the consumers"
( cd "$HERE/fixture-signer" && cargo build --release ) || fail "the fixture signer did not build"
SIGNER=${CARGO_TARGET_DIR:-$HERE/fixture-signer/target}/release/rivet-fixture-signer
[ -x "$SIGNER" ] || fail "no fixture signer at $SIGNER"

# **`ble-second-consumer` is the same program under a second name**, and that is
# the point rather than a shortcut: what the supervisor refuses is a CONTEST for
# one endpoint, and using one binary means the only difference between stage 1,
# where `ble-listen` is granted the socket, and stage 1b, where it is not, is
# that a second application claims it too.
"$SIGNER" ble-listen "$OUT/ble-listen" "$WORK/ble-listen.manifest.toml" \
    "$WORK/ble-listen.header" "$WORK/trust" >/dev/null || fail "could not sign ble-listen"
"$SIGNER" ble-second-consumer "$OUT/ble-listen" "$WORK/ble-listen.manifest.toml" \
    "$WORK/ble-second.header" "$WORK/trust" >/dev/null || fail "could not sign ble-second-consumer"
"$SIGNER" ble-backpressure "$OUT/ble-backpressure" "$WORK/ble-backpressure.manifest.toml" \
    "$WORK/ble-backpressure.header" "$WORK/trust" >/dev/null || fail "could not sign ble-backpressure"

# --- the two documents ------------------------------------------------------
#
# **`enabled = true` at BOOT, and not a reload.** Every start in `ADR-041` §8.3
# was `rivet apply edit` plus `rivet apply reload` on a machine that was already
# up, which is why §8.4 kept *"whether a scan survives a reboot into a scan"* on
# the list. These documents are read by PID 1 before anything has run.
{
    printf 'version = 1\n'
    printf '\n[services.rivet-bluetoothd]\nenabled = true\n'
    printf '\n[services.ble-listen]\nenabled = true\n'
} >"$WORK/stage1.toml"
{
    printf 'version = 1\n'
    printf '\n[services.rivet-bluetoothd]\nenabled = true\n'
    printf '\n[services.ble-listen]\nenabled = true\n'
    printf '\n[services.ble-second-consumer]\nenabled = true\n'
} >"$WORK/stage1b.toml"
{
    printf 'version = 1\n'
    printf '\n[services.rivet-bluetoothd]\nenabled = true\n'
    printf '\n[services.ble-backpressure]\nenabled = true\n'
} >"$WORK/stage2.toml"

# **The kernel is `ADR-024`'s own and is mounted, not built.**
# `tools/container/qemu.sh` exports `RIVET_KERNEL`; `exec.sh` mounts the same
# volume at `/br` and does not, so this fills it in rather than making the
# caller remember which wrapper sets what.
if [ -z "${RIVET_KERNEL:-}" ] && [ -f /br/images/Image ]; then
    RIVET_KERNEL=/br/images/Image
    export RIVET_KERNEL
fi

# **Bounded, because PID 1 parks and nothing here ever exits** — `smoke.sh`'s
# device and its reasoning: `run.sh` execs `qemu-system-aarch64` with no
# intervening fork, so `timeout` signals QEMU itself. A non-zero exit from the
# timeout is the expected end of a good run and is not read as a verdict.
STAGE1_SECONDS=${RIVET_PROOF_STAGE1_SECONDS:-95}
STAGE1B_SECONDS=${RIVET_PROOF_STAGE1B_SECONDS:-60}
STAGE2_SECONDS=${RIVET_PROOF_STAGE2_SECONDS:-90}
LOG1=${RIVET_PROOF_LOG1:-$WORK/stage1.log}
LOG1B=${RIVET_PROOF_LOG1B:-$WORK/stage1b.log}
LOG2=${RIVET_PROOF_LOG2:-$WORK/stage2.log}

cleanup() {
    if [ "${RIVET_PROOF_KEEP:-no}" = yes ]; then
        echo "adr-041-proof: leaving $WORK in place (RIVET_PROOF_KEEP=yes)" >&2
    else
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

# Grep helpers. **Not anchored at the start of a line**: the supervisor captures
# an application's stdout (`ADR-022` §4) and re-emits each line as a journal
# record, so a probe's words arrive inside a JSON `msg` field. An anchored grep
# found nothing and reported a probe silent on a boot where it had said
# everything — `tools/qemu/adr-042-proof.sh` records the same trap.
LOG=
present() {
    grep -aqF "$1" "$LOG" || {
        echo "--- the last 40 lines of $LOG ---" >&2
        tail -40 "$LOG" >&2
        fail "$2
    expected to find: $1"
    }
}
absent() {
    if grep -aqF "$1" "$LOG"; then
        echo "--- what said it ---" >&2
        grep -aF "$1" "$LOG" | head -5 >&2
        fail "$2"
    fi
}

# ===========================================================================
# STAGE 1 — AN ADVERTISEMENT CROSSES THE ENDPOINT
# ===========================================================================
STAGE="1 an advertisement crosses the endpoint"
# **A trickle and not a burst, and it lasts longer than a minute.** Two reasons,
# and both are about not writing a race into a proof:
#
# * Nothing orders a consumer after its provider — `rivet_sdk::bluetooth::
#   advertisements` says so in its own refusal — so a fixture that emptied its
#   whole plan at once could finish before the application had connected.
# * `examples/probes/ble-listen`'s roll-up is written every SIXTY SECONDS **and
#   only when an advertisement arrives**, so a stream that stopped at twenty
#   seconds would never produce the line that says how many were heard, from how
#   many devices, and how many did not parse — which is the shape §8.3.2 reports
#   from the board and the one worth being able to compare against.
#
# 3,000 at 25 ms apart is about 76 seconds of traffic at 40 a second.
say "STAGE 1: booting, bounded at ${STAGE1_SECONDS}s, console to $LOG1"
set +e
timeout -k 2 "$STAGE1_SECONDS" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --bluetoothd "$OUT/rivet-bluetoothd" \
    --vhci "$OUT/rivet-vhci-fixture" \
    --vhci-plan count=3000,gap_us=25000,delay_ms=1000 \
    --document "$WORK/stage1.toml" \
    --app "ble-listen:$OUT/ble-listen:$WORK/ble-listen.manifest.toml:$WORK/ble-listen.header" \
    --trust "$WORK/trust" \
    >"$LOG1" 2>&1 </dev/null
set -e
LOG=$LOG1

# --- the controller exists, and the kernel agrees it does -------------------
present "the kernel registered hci" \
    "the fixture never created a controller. Either CONFIG_BT_HCIVHCI is not in this
    kernel — tools/buildroot/linux/qemu-virt.config asks for it — or /dev/vhci was
    not there, in which case the fixture's own line above says which."
present '"msg":"the management socket is bound' \
    "rivet-bluetoothd never bound the management socket, so this kernel has no
    Bluetooth subsystem and nothing below could have happened."
# **Nothing here orders the fixture before the daemon**, and neither should it:
# the fixture waits for devtmpfs and the supervisor starts services when it is
# ready, so which of them is first is the scheduler's business. Both orders are
# correct and this asserts whichever one happened.
#
# If the daemon looked first it must have gone on to notice the controller
# APPEARING, which is `ADR-041` §6.4's whole argument — it blocks on the
# management socket for `Index Added` rather than exiting or spinning. A run
# where it said there was no controller and never said otherwise is that
# argument failing.
if grep -aqF '"msg":"this machine has NO Bluetooth controller' "$LOG"; then
    present '"msg":"a Bluetooth controller appeared' \
        "the daemon reported no controller and then never noticed one appearing.
    ADR-041 §6.4 has it block on the management socket for Index Added rather
    than exit or spin, and this is that path failing."
    say "STAGE 1: the daemon looked before the fixture had registered one, and §6.4's"
    say "         wait-for-Index-Added path is what brought it back"
fi

# --- the scan setup reached a real kernel, in the order the daemon says -----
#
# `0x200b` and `0x200c` are LE Set Scan Parameters and LE Set Scan Enable, and
# **only the daemon sends them**: the kernel's own interrogation of this
# controller is the ten commands the fixture's header lists and neither is among
# them. Their arrival is the difference between a scan that was set up and one
# that was assumed.
present "<- HCI command 0x200b" \
    "the controller was never given LE scan parameters, so cmd/rivet-bluetoothd's
    five-command sequence did not reach it."
present "<- HCI command 0x200c" "the scan was never enabled on the controller."
present "the host set LE scanning ON" "the scan enable did not carry enable = 1."
present '"msg":"SCANNING' \
    "the daemon never reported SCANNING, so hci::Sequence did not reach
    Progress::Scanning against a real controller."
present "INJECTED " "the fixture never wrote any advertisement into the kernel."

# --- and it scanned holding nothing ------------------------------------------
#
# `ADR-041` §6.6: once the user channel is bound the daemon empties every
# capability set and reads them back. These are the kernel's own answers, in
# `/proc/<pid>/status`'s spelling, journalled by the daemon from `capget` and
# `prctl` — under its own uid and the image-service ruleset, as it runs on a
# card.
present '"msg":"the Bluetooth controller is bound, and this daemon gave up every capability' \
    "the daemon never said it gave up its capabilities after the bind."
for _set in CapInh CapPrm CapEff CapBnd CapAmb; do
    present "\"$_set\":\"0000000000000000\"" \
        "the daemon's $_set was not empty after the bind (ADR-041 §6.6)."
done
present '"NoNewPrivs":1' "the daemon was scanning without NoNewPrivs set."

# --- AND AN APPLICATION READ THEM ------------------------------------------
present "ble-listen: reading the advertisement stream at" \
    "ble-listen never opened the endpoint, so nothing consumed it."
# The three synthetic advertisers, each identified by the address the FIXTURE
# composed — least significant byte first on the wire, printed in the order a
# person writes it. A byte-order mistake here does not fail: it produces a
# plausible address belonging to nobody, which is why all three are checked
# against what was written rather than merely counted.
present "AA:BB:CC:00:00:01" "the first advertiser's address never reached the consumer."
present "AA:BB:CC:00:00:02" "the second advertiser's address never reached the consumer."
present "AA:BB:CC:00:00:03" "the third advertiser's address never reached the consumer."
# **The AD structures, split, on a detail line.** `01:06` is Flags and
# `09:52495645542d46495854555245` is AD type 0x09 — Complete Local Name — with
# `RIVET-FIXTURE` in it, which is the structure a Govee used to name itself on
# the board (`ADR-041` §8.3.2). Asserting the hex rather than the rendered name
# checks the AD walk in `rivet_core::bluetooth::ad_structures`: a walk that
# mislaid one length byte would produce a plausible pair of structures out of
# the same bytes.
present "[01:06 09:52495645542d46495854555245]" \
    "the AD structures did not survive the crossing. 0x09 is Complete Local Name
    and the hex is RIVET-FIXTURE; a walk that lost a length byte would still
    produce structures, so the bytes are checked and not merely their count."
present "rssi -42dBm" "the signal strength did not survive the crossing."
present "Connectable" "the ADV_IND event type did not survive the crossing."
present "NonConnectable" "the ADV_NONCONN_IND event type did not survive the crossing."

# **Nothing failed to parse**, which is the property `ADR-041` §8.3.2 bought on
# the board with 2,793 lines off the air. Here it is much weaker — the fixture
# and the decoder are two implementations of one format, but the traffic is
# synthetic — and it is still the assertion that fails if the encoder and the
# decoder ever part.
absent "A LINE DID NOT PARSE" \
    "rivet_core::bluetooth's encoder and rivet_sdk::bluetooth's decoder disagree
    about a line one of them wrote."

DETAILED=$(grep -ao 'ble-listen: first [0-9]* - ' "$LOG" | wc -l | tr -d ' ')
[ "$DETAILED" -ge 24 ] || fail "ble-listen printed $DETAILED advertisement(s) in full and its
    DETAIL_COUNT is 24. Fewer than that means it connected too late in the
    fifteen seconds of traffic, or the stream stopped."
say "STAGE 1: $DETAILED advertisements were printed in full by the consuming application"
echo "--- the first three, verbatim ---"
grep -ao 'ble-listen: first [0-9]* - [^"]*' "$LOG" | head -3
echo "--- end ---"

# --- the roll-up, which is §8.3.2's own shape ------------------------------
#
# One line carrying three numbers: how many were heard, from how many devices,
# and how many the decoder refused. The last is the one that matters —
# `rivet_core::bluetooth`'s encoder and `rivet_sdk::bluetooth`'s decoder are two
# implementations of one line format on purpose (`ADR-036` §5), and this is the
# only place in the run where the consumer counts the ones it could not read.
present "device(s), 0 line(s) that did not parse" \
    "either ble-listen never wrote its roll-up — it writes one every sixty seconds
    AND ONLY WHEN AN ADVERTISEMENT ARRIVES, so a stream that stopped early
    produces none, and the plan above is 76 seconds long for that reason — or it
    wrote one that names lines it could not decode."
HEARD=$(grep -ao 'ble-listen: heard [0-9]*' "$LOG" | tail -1 | tr -dc '0-9')
INJECTED1=$(grep -ao 'INJECTED [0-9]*' "$LOG" | tail -1 | tr -dc '0-9')
[ -n "$HEARD" ] || fail "the roll-up carried no count"
[ "$HEARD" -gt 0 ] || fail "the roll-up says $HEARD advertisements were heard"
# **A snapshot, not a tally, and the difference matters.** The roll-up is
# written sixty seconds into a stream that lasts about seventy-six, so it counts
# what had arrived by then. The fixture's own INJECTED line comes at the end of
# the run. Reading the gap between them as loss would be wrong, and the line
# that WOULD say something about loss is the third number above: zero.
say "STAGE 1: the consumer's roll-up at sixty seconds says it had heard $HEARD, of the"
say "         ${INJECTED1:-?} the fixture wrote over the whole run. It is a snapshot of a stream"
say "         still running, not a tally, and the count that matters is the third: 0"
say "         line(s) that did not parse."
grep -ao 'ble-listen: heard [^"]*' "$LOG" | tail -1
grep -ao 'ble-listen: device [^"]*' "$LOG" | tail -3

say "STAGE 1 PASSED: an advertisement crossed the endpoint under emulation, to the"
say "                one application that claimed it"

# ===========================================================================
# STAGE 1b — TWO APPLICATIONS CLAIM THE ENDPOINT, AND NEITHER GETS IT
# ===========================================================================
STAGE="1b two claimants"
# **A boot of its own, because the contest is decided at start.** The supervisor
# counts every enabled definition that claims an unshared endpoint, and with two
# it refuses BOTH: granting it to whichever sorts first would let an application
# installed later take it from the one holding it, just by its name. No
# controller is needed — the refusal is about declarations, and stage 1 already
# showed the endpoint carrying traffic.
say "STAGE 1b: booting, bounded at ${STAGE1B_SECONDS}s, console to $LOG1B"
set +e
timeout -k 2 "$STAGE1B_SECONDS" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --bluetoothd "$OUT/rivet-bluetoothd" \
    --document "$WORK/stage1b.toml" \
    --app "ble-listen:$OUT/ble-listen:$WORK/ble-listen.manifest.toml:$WORK/ble-listen.header" \
    --app "ble-second-consumer:$OUT/ble-listen:$WORK/ble-listen.manifest.toml:$WORK/ble-second.header" \
    --trust "$WORK/trust" \
    >"$LOG1B" 2>&1 </dev/null
set -e
LOG=$LOG1B

CLAIMANTS="and so does every application in [ble-listen, ble-second-consumer]"
present "ble-listen wants rivet-bluetoothd:advertisements, $CLAIMANTS" \
    "ble-listen was not refused with both claimants named. ADR-015 §5.3's grant is a
    chown and a chown names one uid, and an endpoint two applications claim goes to
    neither — so the one that sorts first must be refused too."
present "ble-second-consumer wants rivet-bluetoothd:advertisements, $CLAIMANTS" \
    "ble-second-consumer was not refused with both claimants named."
present "this application declares connects and the connection was refused, so it was \
NOT started" \
    "the refusal did not stop the claimants being started. A program failing at a
    socket nobody granted it is the silent wrong answer that refusal exists to avoid."
REFUSED=$(grep -ac 'declares connects and the connection was refused' "$LOG" || true)
[ "$REFUSED" -ge 2 ] || fail "the supervisor journalled $REFUSED refusal(s) to start, and both
    claimants must have one."
# **Neither opened the endpoint.** Stage 1 is the control: the same binary, the
# same manifest and the same header DID open it when nothing contested it, so
# what was refused here is the contest and not the declaration.
absent "ble-listen: reading the advertisement stream at" \
    "ble-listen opened the endpoint although a second application claims it. The
    contest was decided first-come, which is the rule this machine no longer has."
absent "ble-second-consumer: reading the advertisement stream at" \
    "ble-second-consumer opened the endpoint although it is contested."
grep -ao 'ble-listen wants rivet-bluetoothd:advertisements, [^.]*' "$LOG" | head -1
say "STAGE 1b PASSED: two applications claimed one endpoint, both were refused, and"
say "                 each refusal named both"

# ===========================================================================
# STAGE 2 — A CONSUMER THAT CANNOT KEEP UP
# ===========================================================================
STAGE="2 a consumer that cannot keep up"
say "STAGE 2: booting, bounded at ${STAGE2_SECONDS}s, console to $LOG2"
set +e
timeout -k 2 "$STAGE2_SECONDS" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --bluetoothd "$OUT/rivet-bluetoothd" \
    --vhci "$OUT/rivet-vhci-fixture" \
    --vhci-plan count=20000,gap_us=0,delay_ms=5000 \
    --document "$WORK/stage2.toml" \
    --app "ble-backpressure:$OUT/ble-backpressure:$WORK/ble-backpressure.manifest.toml:$WORK/ble-backpressure.header" \
    --trust "$WORK/trust" \
    >"$LOG2" 2>&1 </dev/null
set -e
LOG=$LOG2

present '"msg":"SCANNING' "the daemon never reached SCANNING on this boot."
present "ble-backpressure: HOLDING " \
    "the consumer never took the endpoint, so nothing was behind and a daemon that
    reported no drops would be reporting correctly."
# **The within-boot control comes FIRST, and without it the drops prove
# nothing.** A daemon writing into a socket that never carried anything would
# report exactly the same drops as one writing into a socket that filled. So the
# consumer reads five advertisements over this connection before it goes deaf,
# and both halves are asserted about one socket, one consumer and one boot —
# `AGENTS.md` §7, and `tools/qemu/adr-042-proof.sh`'s second half is the same
# shape.
present "ble-backpressure: read 5 - " \
    "the consumer never received five advertisements, so nothing here shows the
    granted socket carrying anything and the drops below could be a socket that
    was broken from the start."
present "ble-backpressure: DEAF from here on" \
    "the consumer never stopped reading, so nothing was ever behind."
present "records were DROPPED rather than waited for" \
    "ADR-041 §6.3's drop-and-count path was NOT taken. Either the consumer's socket
    buffer never filled — raise count — or the daemon is blocking on a slow
    consumer, which is the failure that section exists to prevent and which looks
    from here exactly like a quiet success."

INJECTED=$(grep -ao 'INJECTED [0-9]*' "$LOG" | head -1 | tr -dc '0-9')
DROPPED=$(grep -ao '"dropped":[0-9]*' "$LOG" | tail -1 | tr -dc '0-9')
[ -n "$INJECTED" ] || fail "the fixture never said how many it injected"
[ -n "$DROPPED" ] || fail "the daemon reported a drop and no count with it"
[ "$DROPPED" -gt 0 ] || fail "the reported drop count is $DROPPED"
# **The daemon carried on**, which is the property §6.3 exists for: a blocking
# write would have stalled the process reading the controller, and the last
# thing this run would show is the first drop. Anything after it is the loop
# still turning.
[ "$DROPPED" -lt "$INJECTED" ] || fail "the drop count reached everything that was injected,
    so nothing distinguishes this from a socket that accepted nothing at all."
say "STAGE 2 PASSED: five advertisements were read over the granted socket, and then"
say "                $DROPPED were dropped and counted while the fixture wrote $INJECTED."
say "                The drop count is a snapshot at the moment the run was cut short;"
say "                what it establishes is that the daemon dropped rather than blocked."
echo "--- the consumer's five, and then the daemon's own words ---"
grep -ao 'ble-backpressure: read [0-9]* - [^"]*' "$LOG" | head -2
grep -ao '"msg":"an application is not reading[^"]*"' "$LOG" | head -1
grep -ao '"consumer":[0-9]*,"dropped":[0-9]*' "$LOG" | tail -1
echo "--- end ---"

# ===========================================================================
# STAGE 3 — THE CONTROLLER IS UNPLUGGED AND PLUGGED BACK IN
# ===========================================================================
STAGE="3 unplug and replug"
# `ADR-041` §6.6. A daemon holding nothing cannot bind the next controller, so
# it exits and the supervisor brings it back holding CAP_NET_ADMIN. The fixture
# closes /dev/vhci after its first round — `hci_unregister_dev`, which is what
# pulling a dongle does — waits, and creates a new controller.
{
    printf 'version = 1\n'
    printf '\n[services.rivet-bluetoothd]\nenabled = true\n'
} >"$WORK/stage3.toml"
STAGE3_SECONDS=${RIVET_PROOF_STAGE3_SECONDS:-75}
LOG3=${RIVET_PROOF_LOG3:-$WORK/stage3.log}
say "STAGE 3: booting, bounded at ${STAGE3_SECONDS}s, console to $LOG3"
set +e
timeout -k 2 "$STAGE3_SECONDS" "$HERE/run.sh" \
    "$OUT/rivet-initd" \
    --supervisor "$OUT/rivet-sysd" \
    --bluetoothd "$OUT/rivet-bluetoothd" \
    --vhci "$OUT/rivet-vhci-fixture" \
    --vhci-plan count=40,gap_us=25000,delay_ms=500,unplug=1,away_ms=3000 \
    --document "$WORK/stage3.toml" \
    >"$LOG3" 2>&1 </dev/null
set -e
LOG=$LOG3

present "UNPLUGGED after round 0" "the fixture never unplugged its controller."
present "PLUGGED BACK IN, round 1" "the fixture never plugged a controller back in."
if ! grep -aqE '"msg":"the Bluetooth controller (went away|refused a read)' "$LOG"; then
    tail -40 "$LOG" >&2
    fail "the daemon never noticed its controller going away."
fi
EXITED4=$(grep -aF '"status":"exit 4"' "$LOG" | grep -acF 'rivet-bluetoothd' || true)
[ "$EXITED4" -ge 1 ] || fail "the supervisor never recorded rivet-bluetoothd exiting 4, the
    exit that means its controller is gone."
DROPS=$(grep -acF '"msg":"the Bluetooth controller is bound, and this daemon gave up every capability' "$LOG" || true)
SCANS=$(grep -acF '"msg":"SCANNING' "$LOG" || true)
[ "$DROPS" -ge 2 ] || fail "the daemon gave up its capabilities $DROPS time(s); a second run
    after the replug must bind the new controller and drop again."
[ "$SCANS" -ge 2 ] || fail "the daemon reported SCANNING $SCANS time(s); the replugged
    controller was never scanned."
absent "giving up; the service is marked failed" \
    "the supervisor gave up on a controller that was unplugged once."
say "STAGE 3 PASSED: the controller went away, rivet-bluetoothd exited 4, the supervisor"
say "                started it again, and it took the new controller, gave up every"
say "                capability again and scanned ($DROPS drops, $SCANS scans)."

echo
say "PASSED — on four boots, with no radio anywhere:"
say "  an advertisement composed by a virtual controller crossed the whole path —"
say "  controller, kernel, user channel, daemon, granted socket, application —"
say "  read by a daemon holding no capability;"
say "  two applications claiming that endpoint were both refused and not started;"
say "  a consumer that would not read had records dropped, counted and reported,"
say "  and the daemon carried on reading;"
say "  a controller unplugged and plugged back in was taken again by a restart."
say "It settles NOTHING about a radio, about the air, or about power: see this"
say "file's header and ADR-041 §8.4."
