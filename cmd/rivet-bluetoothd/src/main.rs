//! `rivet-bluetoothd` — **a Rivet reads BLE advertisements.**
//!
//! `ADR-041`, ruled 2026-09-05 (`.rulings/` R-144):
//!
//! > tier 1 only. BLE observer — reading advertisements. Tier 2 (GATT central)
//! > and tier 3 (Classic HID) are deferred, not rejected.
//!
//! ```text
//! rivet-bluetoothd
//! ```
//!
//! No arguments. It is not told which adapter to use — it asks the kernel — and
//! it is not told where its socket is, because it is handed one.
//!
//! # What it is FOR, in one paragraph
//!
//! An HCI user-channel socket is **exclusive**: one process holds the adapter
//! and a second gets `EUSERS`. So an SDK module that every application called
//! would work for exactly one application and then silently stop for the
//! second, which is the kind of defect that is discovered in production. This
//! daemon is the arbiter, in the same shape `rivet-launcher` already has for
//! the display (`ADR-015` §6.2, `ADR-036` §3.1): **the platform holds the
//! scarce thing and hands applications what they need.**
//!
//! What an application needs is a line per advertisement, and it gets it by
//! declaring `connects = ["rivet-bluetoothd:advertisements"]` — the mechanism
//! that already exists (`ADR-015` §5.3). There is **no new device class**;
//! `ADR-041` §6.2 argues why not.
//!
//! # What is deliberately not here
//!
//! * **No BlueZ and no D-Bus.** Tier 1 needs neither, and the 2026-09-05 study
//!   proving both build and run is exactly the trap: shipping a message bus
//!   *because it turned out to be possible* would be the wrong lesson from a
//!   good measurement (R-144.1 item 1).
//! * **No connections, no pairing, no GATT, no Classic.** R-144.1 item 4:
//!   a daemon built ready for a deferred tier carries untested surface for a
//!   capability nobody ruled.
//! * **No vendor decoding.** The maintainer's Govee sensors are why this exists
//!   and nothing here knows what a Govee is. `rivet_core::bluetooth` says why.
//! * **No filter.** It fans out everything it hears; `crate::stream` argues it.
//!
//! # Nothing here has run on hardware
//!
//! `AGENTS.md` §6. Every measurement behind this binary is QEMU with
//! `CONFIG_BT_HCIVHCI` — a controller with no radio — or a build. Whether the
//! CYW43455 attaches over serdev, answers a reset, and hears anything is the
//! board's to settle and no emulator's: `ADR-026` §4 is the table, and
//! `ADR-041` §8 is the list of what one boot would close.

mod hci;
mod journal;
mod mgmt;
mod privilege;
mod run;
mod sockets;
mod stream;

/// `ADR-022` §3.2's `svc`, and `ADR-017` §2.1's service name. One spelling.
pub const SELF_NAME: &str = rivet_core::bluetooth::PROVIDER;

/// `ADR-022` §3.2's `actor`.
pub const ACTOR: &str = "bluetoothd";

/// Where the boot id is read from, for `ADR-022` §3.2's `boot`.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// What an exit code means, for `ADR-017` §3.5's restart policy.
pub mod exit {
    /// It was asked to stop and did.
    pub const STOPPED: i32 = 0;
    /// The socket the supervisor was supposed to hand over was not there.
    ///
    /// **A definition fault, not a machine one**, so it is distinct: no amount
    /// of restarting fixes a `provides` that is missing from the definition.
    pub const NO_SOCKET: i32 = 2;
    /// The kernel has no Bluetooth subsystem.
    ///
    /// Every Rivet built before `ADR-041` is in this state, and so may the x86
    /// target be for ever. **It is reported and it does not loop**: see
    /// [`crate::run`] for why this daemon stays up instead.
    pub const NO_SUBSYSTEM: i32 = 3;
    /// The controller went away, or could not be taken or put into scanning.
    ///
    /// **The one exit the supervisor is meant to undo.** Once its controller is
    /// bound this daemon holds no capability, so it cannot bind another; it
    /// exits, and `on-failure` starts it again holding `CAP_NET_ADMIN`.
    pub const CONTROLLER_GONE: i32 = 4;
    /// The capabilities could not all be given up once the controller was
    /// bound. It does not scan holding what it said it would drop.
    pub const PRIVILEGE: i32 = 5;
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", usage());
        return;
    }
    if let Some(unexpected) = argv.first() {
        // Before the journal exists, so a plain line on stderr. The supervisor
        // captures it.
        eprintln!("{SELF_NAME}: `{unexpected}` is not an argument this program takes");
        eprint!("{}", usage());
        std::process::exit(exit::NO_SOCKET);
    }

    // `ADR-022` §3.2's `boot`. A machine whose boot id cannot be read still has
    // a usable journal — ordering within a run is `mono` — so this degrades to
    // a marker rather than refusing to start, exactly as `cmd/rivet-timed`
    // does. The marker is a word so a reader knows it is an absence.
    let boot = std::fs::read_to_string(BOOT_ID_PATH)
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let mut journal = journal::Journal::new(boot, std::process::id(), ACTOR.to_string());

    std::process::exit(run::run(&mut journal));
}

fn usage() -> String {
    format!(
        "usage: {SELF_NAME}\n\n\
         Reads BLE advertisements and hands them to the one application that declares\n\
         connects = [\"{}:{}\"]. Takes no arguments: the adapter is asked of the\n\
         kernel and the socket is handed over by the supervisor (ADR-041).\n",
        rivet_core::bluetooth::PROVIDER,
        rivet_core::bluetooth::ENDPOINT
    )
}
