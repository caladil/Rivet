//! **A Bluetooth controller with no radio**, for `tools/qemu/run.sh --vhci`.
//!
//! `ADR-041` §8.4, and `tools/buildroot/linux/qemu-virt.config`'s
//! `CONFIG_BT_HCIVHCI` block, which was written for exactly this and had no
//! caller until now:
//!
//! > `drivers/bluetooth/hci_vhci.c` registers an HCI controller when a process
//! > opens `/dev/vhci` and writes `HCI_VENDOR_PKT`; the process on the other
//! > end of that descriptor **is** the controller.
//!
//! So this program is the controller. It answers the kernel's own start-up
//! interrogation, then answers `rivet-bluetoothd`'s five-command scan setup,
//! then **writes `LE Advertising Report` events into the kernel** — which is
//! the only way an advertisement can exist on a machine with no radio.
//!
//! # Why it is also PID 1 for about a millisecond
//!
//! A controller has to be a process, and this harness has nowhere to put one.
//! `cmd/rivet-initd` spawns the supervisor and nothing else; the supervisor
//! starts services from `cmd/rivet-sysd/src/definitions.rs`, which is product
//! code that must not learn about a test fixture; and an *application* runs
//! confined, under a uid of its own, where `/dev/vhci` — `crw------- root
//! root`, on `ADR-024`'s devtmpfs with no udev — is unopenable by design.
//!
//! So `run.sh --vhci` boots this as `rdinit=`. It starts one child of itself
//! and then **`exec`s the real PID 1**, which therefore *is* PID 1 and is
//! reached with the argv it would have had. What the guest gains over an
//! ordinary boot is exactly one extra process; nothing about Rivet's own
//! start-up is intercepted, wrapped or reordered.
//!
//! # What it does NOT settle, and the list is short and important
//!
//! * **Nothing about a radio.** `ADR-026` §4's right-hand column. Whether the
//!   CYW43455 attaches, answers a reset and hears a neighbour's sensor is the
//!   board's to say, and `ADR-041` §8.1–§8.3 is where it said it.
//! * **Nothing about the AIR.** Every byte below was composed by this program.
//!   What crossing the endpoint proves here is the *path* — controller, kernel,
//!   user channel, daemon, socket, application — not that anybody's hardware
//!   emits what is asserted.
//! * **Nothing about timing.** A burst written as fast as a `write(2)` loop can
//!   go is not 18 advertisements a second from 29 devices in a house.
//!
//! # Usage
//!
//! ```text
//! rivet-vhci-fixture <plan> <real-init> [init args...]   # as rdinit=
//! rivet-vhci-fixture --controller <plan>                 # the child it starts
//! ```
//!
//! `<plan>` is comma-separated `key=value` and carries no spaces, because it
//! travels on the kernel command line. See [`Plan`].

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::process::CommandExt as _;
use std::process::Command;
use std::time::{Duration, Instant};

/// Where `tools/qemu/run.sh` packs this program.
///
/// **Read out of this file by that script** rather than written there, which is
/// the device every other path in `run.sh` uses: a harness that packed a binary
/// where nothing looks for it fails as a boot that never happens, and the
/// symptom names no file.
pub const FIXTURE_PATH: &str = "/system/bin/rivet-vhci-fixture";

/// The character device `hci_vhci.c` registers, on `ADR-024`'s devtmpfs.
///
/// **It does not exist when this program starts.** `rdinit=` runs before PID 1
/// has mounted anything, and `/dev` in the initramfs holds one node
/// (`run.sh` makes `console` and nothing else). `rivet_core::mount`'s stage 0
/// plan is what puts devtmpfs here, so the child waits rather than assuming.
pub const VHCI_PATH: &str = "/dev/vhci";

/// The prefix on every line this program writes, so a proof script can find
/// them in a console shared with the kernel, PID 1 and the supervisor.
const SAY: &str = "vhci-fixture:";

// --- the wire, from the Core specification and include/net/bluetooth/hci.h ---

/// `HCI_COMMAND_PKT`, the type byte on a command from the host.
const PKT_COMMAND: u8 = 0x01;
/// `HCI_EVENT_PKT`, the type byte on an event from the controller.
const PKT_EVENT: u8 = 0x04;
/// `HCI_VENDOR_PKT`. `vhci_get_user` takes exactly two bytes of it — this and
/// one opcode — and creates a controller.
const PKT_VENDOR: u8 = 0xFF;

/// The `opcode` byte of the vendor packet that creates the device.
///
/// **Zero, and every bit of that matters.** In Linux 6.12's
/// `__vhci_create_device`, bit 6 is `HCI_QUIRK_EXTERNAL_CONFIG` and bit 7 is
/// `HCI_QUIRK_RAW_DEVICE`; bits 2–5 are reserved and a non-zero one is
/// `EINVAL`. **Either quirk would make the controller useless here**, and not
/// obviously: `hci_register_dev` marks a raw device `HCI_UNCONFIGURED`, an
/// unconfigured device is skipped by `mgmt`'s Read Index List, and
/// `mgmt_index_added` returns early for a raw one — so the daemon would find no
/// adapter on a machine that has one. `docs/design-candidates/Bluetooth.md`
/// §4.2 measured the same wall from the other side, as `HCIDEVUP` answering
/// `EOPNOTSUPP`.
const VHCI_CREATE_PRIMARY: u8 = 0x00;

/// `HCI_EV_CMD_COMPLETE`.
const EV_COMMAND_COMPLETE: u8 = 0x0E;
/// `HCI_EV_LE_META`.
const EV_LE_META: u8 = 0x3E;
/// `HCI_EV_LE_ADVERTISING_REPORT`, the subevent code.
const SUBEV_LE_ADVERTISING_REPORT: u8 = 0x02;

/// `HCI_OP_READ_LOCAL_VERSION`.
const OP_READ_LOCAL_VERSION: u16 = 0x1001;
/// `HCI_OP_READ_LOCAL_FEATURES`.
const OP_READ_LOCAL_FEATURES: u16 = 0x1003;
/// `HCI_OP_READ_LOCAL_COMMANDS`.
const OP_READ_LOCAL_COMMANDS: u16 = 0x1002;
/// `HCI_OP_READ_BD_ADDR`.
const OP_READ_BD_ADDR: u16 = 0x1009;
/// `HCI_OP_LE_READ_LOCAL_FEATURES`.
const OP_LE_READ_LOCAL_FEATURES: u16 = 0x2003;
/// `HCI_OP_LE_READ_BUFFER_SIZE`.
const OP_LE_READ_BUFFER_SIZE: u16 = 0x2002;
/// `HCI_OP_LE_READ_SUPPORTED_STATES`.
const OP_LE_READ_SUPPORTED_STATES: u16 = 0x201C;
/// `HCI_OP_LE_SET_SCAN_ENABLE`, which is how this program knows the daemon has
/// finished setting up and an advertisement would now be delivered.
const OP_LE_SET_SCAN_ENABLE: u16 = 0x200C;

/// The most one HCI packet can be: three header bytes and a `u8` length.
const PACKET_MAX: usize = 260;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let me = argv
        .first()
        .filter(|a| a.starts_with('/'))
        .cloned()
        .unwrap_or_else(|| FIXTURE_PATH.to_string());
    let rest = &argv[1.min(argv.len())..];

    match rest.first().map(String::as_str) {
        Some("--controller") => {
            let plan = Plan::parse(rest.get(1).map_or("", String::as_str));
            controller(&plan);
            // The controller never finishes on a healthy run — it is the
            // machine's Bluetooth hardware for as long as the machine is up.
            // Reaching here means it gave up and said why.
            std::process::ExitCode::FAILURE
        }
        Some(plan_text) => {
            let Some(init) = rest.get(1) else {
                eprintln!("{SAY} usage: {me} <plan> <real-init> [init args...]");
                return std::process::ExitCode::FAILURE;
            };
            start(&me, plan_text, init, &rest[2..])
        }
        None => {
            eprintln!("{SAY} usage: {me} <plan> <real-init> [init args...]");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Start the controller, then become the real PID 1.
///
/// **The order is the whole of it.** The child is started first so that a
/// failure to start one is reported on a console that still belongs to this
/// program; `exec` after it never returns, and anything printed here after the
/// `exec` call is a diagnosis of the `exec` itself.
fn start(me: &str, plan: &str, init: &str, init_args: &[String]) -> std::process::ExitCode {
    match Command::new(me).arg("--controller").arg(plan).spawn() {
        Ok(child) => println!("{SAY} the virtual controller is process {}", child.id()),
        Err(e) => {
            // Not fatal. A boot with no controller is a boot this harness can
            // still read — it is what every QEMU run before this option was
            // — and refusing to hand over to PID 1 would turn a fixture
            // failure into a machine that never came up.
            eprintln!("{SAY} COULD NOT START THE CONTROLLER ({e}). This boot has no adapter");
        }
    }
    println!("{SAY} handing this process to {init}");
    let e = Command::new(init).args(init_args).exec();
    eprintln!("{SAY} could not exec {init}: {e}. THERE IS NO PID 1 ON THIS MACHINE");
    std::process::ExitCode::FAILURE
}

/// What to inject once the scan is on.
///
/// Every field has a default that produces a controller which is present, is
/// found, and hears nothing — the state of a board in a lead box, and a useful
/// one to be able to boot.
#[derive(Debug, Clone, Copy)]
struct Plan {
    /// How many advertisements to write. `0` injects none.
    count: u64,
    /// Microseconds between them. `0` writes them as fast as the loop can.
    gap_us: u64,
    /// How long to wait after the scan is enabled before the first one.
    delay_ms: u64,
    /// How many seconds to wait for `/dev/vhci` to appear.
    wait_s: u64,
    /// How many times to **unplug and plug back in**: after the advertisements
    /// of a round are written, close `/dev/vhci` — which unregisters the
    /// controller, exactly as pulling a dongle does — wait `away_ms`, and
    /// create a new one. `0`, the default, keeps the one controller for the
    /// whole boot.
    unplug: u64,
    /// How long the controller stays away on each unplug.
    away_ms: u64,
}

impl Plan {
    fn parse(text: &str) -> Self {
        let mut plan = Self {
            count: 0,
            gap_us: 0,
            delay_ms: 250,
            wait_s: 60,
            unplug: 0,
            away_ms: 2000,
        };
        for field in text.split(',').filter(|f| !f.is_empty()) {
            let Some((key, value)) = field.split_once('=') else {
                eprintln!("{SAY} `{field}` in the plan is not key=value and was ignored");
                continue;
            };
            let Ok(number) = value.parse::<u64>() else {
                eprintln!("{SAY} `{field}`: `{value}` is not a number and the field was ignored");
                continue;
            };
            match key {
                "count" => plan.count = number,
                "gap_us" => plan.gap_us = number,
                "delay_ms" => plan.delay_ms = number,
                "wait_s" => plan.wait_s = number,
                "unplug" => plan.unplug = number,
                "away_ms" => plan.away_ms = number,
                _ => eprintln!("{SAY} `{key}` is not a plan key and was ignored"),
            }
        }
        plan
    }
}

/// Be the controller for the whole boot, unplugging and plugging back in as
/// many times as the plan says.
fn controller(plan: &Plan) {
    println!("{SAY} plan: {plan:?}");
    for round in 0..=plan.unplug {
        if round > 0 {
            std::thread::sleep(Duration::from_millis(plan.away_ms));
            println!("{SAY} PLUGGED BACK IN, round {round}");
        }
        let last = round == plan.unplug;
        if !one_controller(plan, last) {
            return;
        }
        if !last {
            // `one_controller` returned and dropped the descriptor, which is
            // what `hci_vhci.c` turns into `hci_unregister_dev`.
            println!("{SAY} UNPLUGGED after round {round}: /dev/vhci is closed");
        }
    }
}

/// Open the transport, create one controller, and be it.
///
/// Returns `true` when it stopped because the round is over and the plan asks
/// for an unplug, `false` when it gave up. With `last` set it never returns
/// on a healthy run.
fn one_controller(plan: &Plan, last: bool) -> bool {
    let Some(mut vhci) = open_vhci(plan.wait_s) else {
        return false;
    };

    // Two bytes: the vendor packet type and the create opcode. `vhci_get_user`
    // refuses anything shorter than two and anything with bytes left over.
    if let Err(e) = vhci.write_all(&[PKT_VENDOR, VHCI_CREATE_PRIMARY]) {
        eprintln!("{SAY} the kernel refused to create a controller: {e}");
        return false;
    }

    let mut scanning = false;
    let mut buf = [0u8; PACKET_MAX];
    loop {
        let n = match vhci.read(&mut buf) {
            Ok(0) => {
                eprintln!("{SAY} {VHCI_PATH} reached end of file, so this controller is gone");
                return false;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("{SAY} {VHCI_PATH} refused a read: {e}");
                return false;
            }
        };
        let packet = &buf[..n];
        match packet.first().copied() {
            // The kernel's answer to the create above: `0xff`, the opcode it
            // was given, and the controller's index.
            Some(PKT_VENDOR) if n >= 4 => {
                println!(
                    "{SAY} the kernel registered hci{} — this process IS that controller",
                    u16::from_le_bytes([packet[2], packet[3]])
                );
            }
            Some(PKT_COMMAND) if n >= 4 => {
                let opcode = u16::from_le_bytes([packet[1], packet[2]]);
                let params = &packet[4..];
                // **Every command, named.** Fifteen or so lines on a boot — the
                // kernel's start-up interrogation and then the daemon's five —
                // and they are the evidence that `cmd/rivet-bluetoothd`'s
                // `hci::ORDER` reached a controller in the order that file
                // says. A proof that only showed the advertisements could not
                // tell a scan that was set up from one that was assumed.
                println!("{SAY} <- HCI command 0x{opcode:04x} ({} parameter byte(s))", params.len());
                if let Err(e) = vhci.write_all(&complete(opcode, params)) {
                    eprintln!("{SAY} could not answer HCI opcode 0x{opcode:04x}: {e}");
                    return false;
                }
                // `Scan_Enable` is the last of the daemon's five commands
                // (`cmd/rivet-bluetoothd/src/hci.rs`'s `ORDER`), and the first
                // parameter byte is 1 for on. The kernel's own shutdown path
                // sends the same opcode with 0, which is why the byte is read
                // rather than the opcode alone.
                if opcode == OP_LE_SET_SCAN_ENABLE {
                    let on = params.first() == Some(&0x01);
                    println!("{SAY} the host set LE scanning {}", if on { "ON" } else { "off" });
                    if on && !scanning {
                        scanning = true;
                        inject(&mut vhci, plan);
                        if !last {
                            return true;
                        }
                    } else if !on {
                        scanning = false;
                    }
                }
            }
            _ => eprintln!("{SAY} {n} byte(s) from the kernel that are not a command: {packet:02x?}"),
        }
    }
}

/// Wait for devtmpfs to bring the node into being, then open it read-write.
fn open_vhci(wait_s: u64) -> Option<File> {
    let deadline = Instant::now() + Duration::from_secs(wait_s);
    let mut said = false;
    loop {
        match OpenOptions::new().read(true).write(true).open(VHCI_PATH) {
            Ok(file) => {
                println!("{SAY} opened {VHCI_PATH}");
                return Some(file);
            }
            Err(e) => {
                if !said {
                    said = true;
                    println!("{SAY} waiting for {VHCI_PATH} ({e})");
                }
                if Instant::now() >= deadline {
                    eprintln!(
                        "{SAY} {VHCI_PATH} DID NOT APPEAR in {wait_s}s ({e}). Either this kernel \
                         was built without CONFIG_BT_HCIVHCI — tools/buildroot/linux/\
                         qemu-virt.config is where it is asked for — or PID 1 never mounted \
                         devtmpfs, in which case the console above says so"
                    );
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// A `Command Complete` for `opcode`, with the return parameters that command
/// needs.
///
/// # Why seven commands get a body and everything else gets one byte
///
/// `hci_cc_func` in `net/bluetooth/hci_event.c` refuses a completion shorter
/// than the handler's `min_len` — *"unexpected cc 0x%4.4x length: %u < %u"* —
/// and fails the whole request, which on the kernel's start-up interrogation
/// means the controller never leaves `HCI_SETUP` and is therefore **never
/// listed by Read Index List**: a machine with an adapter on which
/// `rivet-bluetoothd` correctly reports having none. For an opcode with no
/// handler the same function reads byte 0 as the status, so one byte is right
/// and anything longer earns *"length: %u > %u"* on the console.
///
/// **The seven were measured, not remembered.** A run with every reply padded
/// to 65 bytes made the kernel name each opcode it knows and the length it
/// wanted, and this list is that answer. The whole interrogation is ten
/// commands — `0c03 1003 1001 1009 1002 2003 2002 201c 0c01 2001` — and the
/// three not here (`0c03`, `0c01`, `2001`) take a bare status.
fn complete(opcode: u16, params: &[u8]) -> Vec<u8> {
    let mut rp = vec![0x00u8]; // Status: success.
    match opcode {
        OP_READ_LOCAL_VERSION => {
            // HCI_Version, HCI_Revision, LMP_Version, Manufacturer, LMP_Subversion.
            // 0x0C is Bluetooth 5.3; 0xFFFF is the SIG's "no company assigned",
            // which is the honest answer for a controller that is a program.
            rp.extend_from_slice(&[0x0C, 0x00, 0x00, 0x0C, 0xFF, 0xFF, 0x00, 0x00]);
        }
        OP_READ_LOCAL_FEATURES => {
            // Eight bytes of LMP features. Byte 4 bit 5 is `LMP_NO_BREDR` and
            // bit 6 is `LMP_LE`: **this controller is LE only**, which is what
            // `tools/buildroot/linux/rivet.config` builds a kernel for
            // (`ADR-041` §5.1 turns `CONFIG_BT_BREDR` off) and what tier 1
            // needs. Claiming BR/EDR would have the kernel interrogate a
            // Classic controller that is not here.
            rp.extend_from_slice(&[0, 0, 0, 0, 0x60, 0, 0, 0]);
        }
        OP_READ_LOCAL_COMMANDS => {
            // The 64-byte supported-command bitmap, **all zero on purpose**.
            // The kernel gates most of its start-up on these bits, so a
            // controller that supports nothing optional is interrogated with
            // the fewest commands — and every command not sent is one this
            // fixture cannot answer wrongly. Nothing tier 1 needs is in here:
            // the daemon holds a user channel and asks the controller
            // directly, without consulting the kernel's opinion of it.
            rp.extend_from_slice(&[0u8; 64]);
        }
        OP_LE_READ_LOCAL_FEATURES => {
            // Eight bytes of LE features, all zero: every one of them is
            // optional and nothing tier 1 does is in the list. **Passive
            // scanning is not a feature bit** — it is what an LE controller is
            // for — which is why a controller that claims none of them can
            // still be put into the scan `crate::inject` then feeds.
            rp.extend_from_slice(&[0u8; 8]);
        }
        OP_LE_READ_BUFFER_SIZE => {
            // LE_ACL_Data_Packet_Length and Total_Num_LE_ACL_Data_Packets.
            // **Zero, which is legal and means *use the BR/EDR buffers*** — a
            // controller that will never carry an ACL packet has none to
            // declare. `hci_cc_le_read_buffer_size` refuses a length that is
            // non-zero and below `HCI_MIN_LE_MTU`, and takes zero.
            rp.extend_from_slice(&[0x00, 0x00, 0x00]);
        }
        OP_LE_READ_SUPPORTED_STATES => {
            // The eight-byte state-combination bitmap. Zero: this controller
            // supports no *combination* of roles, which is the truth about a
            // program that only ever receives.
            rp.extend_from_slice(&[0u8; 8]);
        }
        OP_READ_BD_ADDR => {
            // **`BDADDR_ANY`, and it is load-bearing.** `hci_power_on` closes a
            // controller that comes up with no public and no static address,
            // immediately and before it emits Index Added — so by the time the
            // daemon can see this adapter it is already DOWN and its user
            // channel binds first time. With an address the kernel instead
            // leaves it UP for `HCI_AUTO_OFF_TIMEOUT`, and a daemon that looked
            // in that window would get `EBUSY`, release, look again, and spin.
            // It is also true: a controller with no radio has no address.
            rp.extend_from_slice(&[0u8; 6]);
        }
        _ => {}
    }
    let _ = params;
    let mut out = Vec::with_capacity(6 + rp.len());
    out.push(PKT_EVENT);
    out.push(EV_COMMAND_COMPLETE);
    // Parameter total length: Num_HCI_Command_Packets, the opcode, the body.
    out.push(u8::try_from(3 + rp.len()).unwrap_or(u8::MAX));
    out.push(0x01); // Num_HCI_Command_Packets: one more may be sent.
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&rp);
    out
}

/// One synthetic advertiser.
///
/// **Three of them, and none is anybody's.** `AGENTS.md` §8.3 keeps real device
/// addresses out of tracked files, and `ADR-041` §8.3 masks the ones the board
/// heard for that reason. These are `AA:BB:CC:…`, which no manufacturer holds,
/// and the one that names itself says `RIVET-FIXTURE` rather than imitating a
/// sensor — a proof that produced a plausible `GVH5075` line would be a proof
/// somebody could mistake for the board's.
struct Advertiser {
    /// As a person writes it. Reversed on the wire; see [`report`].
    address: [u8; 6],
    /// `Address_Type`: 0 public, 1 random.
    address_type: u8,
    /// `Event_Type`: 0 `ADV_IND`, 2 `ADV_SCAN_IND`, 3 `ADV_NONCONN_IND`.
    event_type: u8,
    rssi: i8,
    /// The length-tagged AD structures, whole.
    data: &'static [u8],
}

/// The three, in the order they are rotated through.
const ADVERTISERS: [Advertiser; 3] = [
    Advertiser {
        address: [0xAA, 0xBB, 0xCC, 0x00, 0x00, 0x01],
        address_type: 0x00,
        event_type: 0x00,
        rssi: -42,
        // Flags (AD type 0x01), then Complete Local Name (0x09)
        // "RIVET-FIXTURE" — the structure `ADR-041` §4.1 records a Govee using
        // to name itself, carrying a name no Govee would send.
        data: &[
            0x02, 0x01, 0x06, 0x0E, 0x09, b'R', b'I', b'V', b'E', b'T', b'-', b'F', b'I', b'X',
            b'T', b'U', b'R', b'E',
        ],
    },
    Advertiser {
        address: [0xAA, 0xBB, 0xCC, 0x00, 0x00, 0x02],
        address_type: 0x01,
        event_type: 0x03,
        rssi: -77,
        // Manufacturer Specific Data (0xFF) under company identifier 0xFFFF,
        // which the SIG reserves and assigns to nobody.
        data: &[0x05, 0xFF, 0xFF, 0xFF, 0xDE, 0xAD],
    },
    Advertiser {
        address: [0xAA, 0xBB, 0xCC, 0x00, 0x00, 0x03],
        address_type: 0x00,
        event_type: 0x02,
        rssi: -95,
        // An empty payload, which is legal and is the shape a parser is most
        // likely to get wrong.
        data: &[],
    },
];

/// Write `plan.count` advertising reports.
fn inject(vhci: &mut File, plan: &Plan) {
    if plan.count == 0 {
        println!("{SAY} the plan asks for no advertisements, so this machine hears nothing");
        return;
    }
    if plan.delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(plan.delay_ms));
    }
    println!(
        "{SAY} INJECTING {} advertisement(s) from {} device(s)",
        plan.count,
        ADVERTISERS.len()
    );
    let started = Instant::now();
    let mut written = 0u64;
    for n in 0..plan.count {
        let who = &ADVERTISERS[usize::try_from(n % 3).unwrap_or(0)];
        if let Err(e) = vhci.write_all(&report(who)) {
            eprintln!("{SAY} the kernel refused advertisement {n}: {e}");
            break;
        }
        written += 1;
        if plan.gap_us > 0 {
            std::thread::sleep(Duration::from_micros(plan.gap_us));
        }
    }
    println!(
        "{SAY} INJECTED {written} advertisement(s) in {} ms",
        started.elapsed().as_millis()
    );
}

/// One `LE Advertising Report` event carrying one report.
///
/// # The layout is record-major and the RSSI is last
///
/// The Core specification writes the parameters as parallel arrays indexed by
/// report, which reads as though every `Event_Type` came first. The kernel's
/// `struct hci_ev_le_advertising_info` is one whole report after another, with
/// the RSSI following the data — `hci_event.c` reads it at
/// `info->data[info->length]`. This composes the kernel's layout because the
/// kernel is what will carry it.
fn report(who: &Advertiser) -> Vec<u8> {
    let mut params = Vec::with_capacity(12 + who.data.len());
    params.push(SUBEV_LE_ADVERTISING_REPORT);
    params.push(0x01); // Num_Reports.
    params.push(who.event_type);
    params.push(who.address_type);
    // **Least significant byte first**, which is the one thing about a BD_ADDR
    // that everybody gets wrong once. Getting it wrong does not fail: it
    // produces a plausible address belonging to nobody.
    for byte in who.address.iter().rev() {
        params.push(*byte);
    }
    params.push(u8::try_from(who.data.len()).unwrap_or(u8::MAX));
    params.extend_from_slice(who.data);
    params.push(who.rssi.to_le_bytes()[0]);

    let mut out = Vec::with_capacity(3 + params.len());
    out.push(PKT_EVENT);
    out.push(EV_LE_META);
    out.push(u8::try_from(params.len()).unwrap_or(u8::MAX));
    out.extend_from_slice(&params);
    out
}
