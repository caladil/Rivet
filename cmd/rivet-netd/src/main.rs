//! Rivet's network bring-up: **every interface the document names, up,
//! addressed and routed over netlink from Rust**, then exit.
//!
//! It is not a daemon. The long-lived network mechanisms are services the
//! supervisor runs beside it, one per interface that needs one, and this
//! program does the one-shot work around them:
//!
//! | Mechanism | Run by | What this program does with it |
//! | :-- | :-- | :-- |
//! | `udhcpc`, per DHCP interface | the supervisor (`ADR-004` §8) | raises the link it broadcasts on, and applies the lease its hook recorded |
//! | `wpa_supplicant`, per wireless interface | the supervisor (`ADR-004` §4) | raises the link, and tells it the network over its control socket |
//!
//! **The supervisor runs this program again** when a lease changes and when it
//! restarts a supplicant, so the address on an interface follows its lease and
//! a restarted supplicant is told its network again. Every netlink write on
//! the machine stays here (`ADR-011`). The firewall is the supervisor's
//! (`ADR-023` §9).
//!
//! ## The document is the authority — ruled 2026-08-23
//!
//! ~~This binary took `--interface`, `--address` and `--gateway`.~~ It now
//! reads the `[network]` table of a configuration document. `ADR-007` §3.3
//! makes that document the statement of what the machine should be, so an
//! address on a service's command line was a second thing that also looked
//! authoritative — `ADR-011` §3. Three shapes were offered and the
//! maintainer took option 2, 2026-08-23.
//!
//! The **path** to the document is still an argument, and that is not the same
//! thing: a path says where to read the statement from, not what the statement
//! is. See [`args`].
//!
//! ## The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`args`] | what `argv` asked for, and every way it can be wrong |
//! | [`render`] | typed outcomes to console lines, and the exit code policy |
//! | `main` | the composition root: read the document, open a socket, run each plan, print, exit. No logic |
//!
//! Everything that decides anything is either in `rivet-config` — which is
//! what a valid document is — or in `rivet-adapter-net`, above its `SendRecv`
//! port. This file is the part that cannot be tested without a kernel and a
//! filesystem, which is why there is so little of it.
//!
//! ## Exit codes
//!
//! `ADR-017` §3.5 restarts a failed service with backoff, so these are policy:
//!
//! | | Means |
//! | :-- | :-- |
//! | 0 | every interface the document names is configured. **Including when there is no carrier** — see [`render::exit_code`] |
//! | 1 | at least one interface refused — including one that asked for a lease |
//! | 2 | the arguments were not usable, or the document could not be read or was refused |
//! | 3 | the netlink socket could not be opened at all |
//!
//! **A document that names no interface exits 0**, loudly. The reasoning is
//! [`render::exit_code`]'s, one level up: the applied configuration is the
//! empty one, which is what was asked for, and exiting non-zero would have
//! `ADR-017` §3.5 restart this binary for ever on a machine whose networking
//! simply has not been configured yet. The console says so in as many words, on
//! the same principle that makes an absent carrier shout.
//!
//! ## The missing half, named rather than faked
//!
//! `ADR-004` §6 has PID 1 read `rivet.toml` from the boot partition on first
//! boot, and `cmd/rivet-initd` does. ~~**Nothing carries that document to a
//! path this binary can read**, because `ADR-014` §2's `rivet-store` —
//! persistence of desired state to `/state/config` — does not exist. Checked
//! 2026-08-23 two ways: it is not a workspace member in the root `Cargo.toml`,
//! and `git ls-files` finds nothing under that name.~~
//!
//! **Corrected later on 2026-08-23: `rivet-store` exists**, and PID 1 writes
//! the provisioning document to `/state/config/machine.toml` on a first boot
//! (`ADR-017` §3.1.9). Struck rather than reworded, per `AGENTS.md` §7.
//!
//! ~~**The half that is still missing is this end of it.** Nothing points this
//! binary at that path. `cmd/rivet-image` **does** install it now, so it is
//! on the card — but `rivet-sysd` does not start it, and nothing hands
//! it a `--document`. So the
//! operator still names a path and puts a document there: a complete and honest
//! Stage 1 story for a board reached over a cable, and still not the first-boot
//! story.~~
//!
//! ~~**Corrected 2026-08-23: it is installed now**, at
//! `/system/bin/rivet-netd`, `0755`, root-owned, in both `/system` slots.
//! **Nothing has changed about what runs it**, so the paragraph above still
//! holds in full.~~
//!
//! **Closed later on 2026-08-23, and the struck text is kept because it was
//! corrected twice in one day** (`AGENTS.md` §7). `rivet-sysd` compiles
//! in a service definition for this binary and starts it with
//! `--document /state/config/machine.toml` — the path `rivet-store` writes,
//! taken from `ConfigStore::machine_path()` rather than spelled a second time.
//! `ADR-017` §4.3 is the definition and its argument.
//!
//! **What that makes this binary, precisely.** It is a service that **runs
//! once**: `restart = on-failure`, so exit 0 leaves it exited and a non-zero
//! exit is retried with `ADR-017` §3.5's backoff and give-up. That is the
//! supervisor's reading of the exit codes below and it is why the table matters
//! — **exit 0 on a document naming no interface is load-bearing**, because a
//! non-zero exit there would have the supervisor start this five times and then
//! mark it failed on a machine whose networking is simply unconfigured.
//!
//! Observed under QEMU 2026-08-23 with `ADR-024`'s own kernel: started by the
//! supervisor as pid 60, `eth0` up, addressed, routed, read back, *configured,
//! carrier present*, exit 0 — and **not** started again. `AGENTS.md` §6: that
//! was `virtio_net` under emulation, not `bcmgenet` on a board, and the
//! document was staged in an initramfs rather than read off an `ext4`
//! `/state`.
//!
//! ## It does not run on the development host
//!
//! `main` on anything but Linux prints a refusal and exits 3. `AGENTS.md` §3 —
//! *stubs refuse; they never fake success*. It compiles there on purpose, so
//! that `cargo check` and this crate's own tests run on the machine the code is
//! written on; what it will not do is claim to have configured a network on a
//! kernel that has no netlink.

mod args;
mod render;

use std::path::Path;

use rivet_config::{Addressing, Document};
use rivet_core::net::StaticAddressing;

use args::Request;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::ExitCode::from(run(&argv))
}

/// The whole program, over a borrowed `argv` so it is drivable from a test.
///
/// Returns the exit code rather than calling `exit`, for the same reason.

/// The loopback interface's name. There is exactly one and it is not
/// configurable.
pub const LOOPBACK: &str = "lo";

/// **Bring `lo` up, before anything reads a document.**
///
/// # Found by the first thing that ever needed it, 2026-08-28
///
/// `rivet-mcpd` tried to bind `127.0.0.1` on a board and got
/// `Address not available (os error 99)`. `lo` was there —
/// `/sys/class/net/lo/flags` read `0x8`, which is `IFF_LOOPBACK` **and not
/// `IFF_UP`** — and nothing on a Rivet machine had ever brought it up.
/// `eth0` read `0x1003` for comparison.
///
/// It had gone unnoticed because nothing needed it. `ADR-008` §— assumes it
/// works — *"what `localhost` means — a property of the loopback interface"* —
/// and the string `"lo"` appeared in this repository exactly once, in a
/// `rivet-core` test asserting the NAME parses. `PLAN.md` §71.
///
/// # Why here, and why before the document
///
/// **Loopback is not configuration.** No `[network]` table names it, no
/// operator should have to, and a machine with no document at all still needs
/// it: `ADR-015` §— prefers a unix socket to loopback for application IPC, but
/// `ADR-035` §1's display application talking MCP to the local control plane,
/// musl's resolver falling back to `127.0.0.1`, and every future local listener
/// all assume it is up.
///
/// So it happens **before** `load`, which means a machine that exits 2 for
/// having no document still comes back with loopback working. That ordering is
/// the point of this function's placement and not an accident of it.
///
/// # It never fails the run
///
/// The same rule the rest of this binary follows: report and carry on. A
/// refusal here is said on the console and does not stop `[network]` from being
/// converged, because an `eth0` that works is worth more than a tidy exit code
/// — and on a headless board it is the only way anyone finds out.
#[cfg(target_os = "linux")]
fn bring_up_loopback() {
    use rivet_adapter_net::link;
    use rivet_adapter_net::port::Sequence;
    use rivet_adapter_net::socket::NetlinkSocket;
    use rivet_core::net::InterfaceName;

    // The name goes through the domain's validator rather than straight to
    // the kernel, for the reason every other interface name here does: one
    // place decides what an interface may be called (`ADR-011`).
    let name = match InterfaceName::new(LOOPBACK) {
        Ok(name) => name,
        Err(e) => {
            say(&render::tagged(&format!(
                "{LOOPBACK} is not a valid interface name: {e}"
            )));
            return;
        }
    };

    let mut socket = match NetlinkSocket::open() {
        Ok(socket) => socket,
        Err(errno) => {
            say(&render::tagged(&format!(
                "cannot open a netlink socket to bring {LOOPBACK} up: {errno}. \
                 127.0.0.1 will not be reachable on this machine"
            )));
            return;
        }
    };
    let mut seq = Sequence::new();

    let index = match link::find(&mut socket, &mut seq, &name) {
        Ok(info) => info.index,
        Err(failure) => {
            say(&render::tagged(&format!(
                "cannot find {LOOPBACK}: {failure:?}. 127.0.0.1 will not be reachable"
            )));
            return;
        }
    };

    match link::set_up(&mut socket, &mut seq, index) {
        Ok(()) => say(&render::tagged(&format!(
            "{LOOPBACK} is up (index {index}); 127.0.0.1 is reachable"
        ))),
        Err(failure) => say(&render::tagged(&format!(
            "could not bring {LOOPBACK} up: {failure:?}. 127.0.0.1 will not be \
             reachable, so anything binding it fails with EADDRNOTAVAIL"
        ))),
    }
}

/// The stub, on the same terms as [`converge`]'s: it refuses rather than
/// reporting a loopback it did not raise.
#[cfg(not(target_os = "linux"))]
fn bring_up_loopback() {
    say(&render::tagged(&format!(
        "refusing to bring {LOOPBACK} up — netlink exists only on Linux, and this binary was \
         built for {}",
        std::env::consts::OS
    )));
}

/// **Say which uid this process runs as and which capabilities it holds**,
/// read out of `/proc/self/status` rather than assumed.
///
/// The supervisor starts this program as uid 0 with its bounding set reduced
/// to the network's capabilities and `NoNewPrivs` set (`ADR-017` §3). The
/// program runs for a moment and exits, so its own journal line is the one
/// durable record of what it actually held. A status that cannot be read is
/// said and does not stop the run.
fn report_privilege() {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        say(&render::tagged(
            "/proc/self/status could not be read, so what this process may do is not reported",
        ));
        return;
    };
    let fields: Vec<String> = status
        .lines()
        .filter(|line| {
            ["Uid:", "CapEff:", "CapBnd:", "NoNewPrivs:"]
                .iter()
                .any(|field| line.starts_with(field))
        })
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    say(&render::tagged(&format!("running with {}", fields.join("; "))));
}

fn run(argv: &[String]) -> u8 {
    let request = match args::parse(argv) {
        Ok(request) => request,
        Err(e) => {
            say(&render::tagged(&e.to_string()));
            say(args::USAGE);
            return 2;
        }
    };

    let path = match request {
        Request::Usage => {
            say(args::USAGE);
            return 0;
        }
        Request::Apply { document } => document,
    };

    // What this process may do, in the kernel's words, before it does any of
    // it — see `report_privilege`.
    report_privilege();

    // **Before the document, deliberately** — see `bring_up_loopback`. A
    // machine that exits 2 below for having no configuration still gets
    // 127.0.0.1, because loopback is a precondition rather than a setting.
    bring_up_loopback();

    let document = match load(&path) {
        Ok(document) => document,
        Err(why) => {
            say(&render::tagged(&why));
            return 2;
        }
    };

    apply(&document)
}

/// Read and parse the document. **The only I/O in this crate that is not a
/// syscall behind `rivet-adapter-net`'s port.**
///
/// The refusal carries the path and `rivet-config`'s own diagnostic, which
/// names a key and a line and column and deliberately never quotes the value it
/// failed on — the same document may hold a wireless passphrase (`ADR-020` §4),
/// and a console is a log.
fn load(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read the configuration document {}: {e}", path.display()))?;
    Document::parse(&text)
        .map_err(|e| format!("{} is not a valid configuration document: {e}", path.display()))
}

/// Converge every interface the document names.
///
/// Two passes rather than one, so the netlink socket is opened once and only if
/// something needs it: a document of nothing but leased interfaces must not
/// report "cannot open a netlink socket" as its problem when its real problem
/// is that DHCP does not exist yet.
fn apply(document: &Document) -> u8 {
    let interfaces = document.network();
    if interfaces.is_empty() {
        // Not a failure and not silence. See the exit-code table above.
        say(&render::tagged(
            "the document's [network] table names no interface, so nothing was configured — \
             this machine has no address from Rivet",
        ));
        return 0;
    }

    let mut code = 0_u8;
    let mut plans: Vec<StaticAddressing> = Vec::new();

    for interface in interfaces {
        // **A wireless interface is JOINED before it is addressed**, and this
        // is above the addressing match because it is true whichever the
        // addressing is: `ADR-004` §2.1's *"almost nothing in the path is
        // link-specific"* cuts both ways — the address does not care how the
        // link came up, and the link has to come up first.
        //
        // A radio with no association is not a link at all: a DHCP DISCOVER
        // goes nowhere and a static address is written to something with no
        // path off the machine.
        if let Some(ssid) = interface.ssid() {
            if let Err(why) = join_wireless(interface.name(), ssid) {
                say(&render::tagged(&format!(
                    "{} could not join {}: {why}",
                    interface.name(),
                    ssid
                )));
                code = code.max(1);
                continue;
            }
        }

        // Matched on the addressing rather than on whether a plan came back,
        // so that a variant added to `Addressing` later fails to compile here
        // instead of being silently skipped.
        match interface.addressing() {
            Addressing::Dhcp => {
                // **The lease becomes a plan and joins the same convergence.**
                // `ADR-004` §2.1: *"almost nothing in the path is
                // link-specific"*. DHCP learns an address differently and asks
                // the kernel for exactly the same thing, so there is one
                // `converge` and not two.
                //
                // **The link must be up for the client to broadcast.** This is
                // `ADR-004` §2.1's *almost*: a static plan raises the link as
                // one step of applying an address it already has, and the
                // client — a supervised service of its own, started beside
                // this program — has to ask before it knows one. A DISCOVER
                // sent on a link that is down is refused by the kernel, and
                // the client sends another a few seconds later.
                if let Err(why) = raise_link(interface.name()) {
                    say(&render::tagged(&format!(
                        "{} could not be raised for DHCP: {why}",
                        interface.name()
                    )));
                    code = code.max(1);
                    continue;
                }
                let present =
                    std::path::Path::new(rivet_adapter_net::dhcp::MECHANISM).is_file();
                let mut read = |path: &str| std::fs::read_to_string(path).ok();
                match rivet_adapter_net::dhcp::current_lease(interface.name(), &mut read, present)
                {
                    // **No lease yet is the ordinary first answer.** The
                    // client is asking; its hook records the lease and
                    // nudges the supervisor, which runs this again. **No
                    // lease after one was held** is a lease withdrawn by
                    // `nak` or `leasefail`, and the address it put on the
                    // interface comes off here (`ADR-004` §8).
                    Ok(None) => {
                        code = code.max(withdraw_stale(interface.name(), None, None));
                        let client =
                            rivet_core::service::dhcp_service_name(interface.name().as_str());
                        for line in render::lease_awaited_lines(interface.name().as_str(), &client)
                        {
                            say(&line);
                        }
                    }
                    Ok(Some(lease)) => {
                        for line in render::lease_taken_lines(&lease) {
                            say(&line);
                        }
                        match lease.to_addressing() {
                            // **Every other address and every other default
                            // route comes off first**, so a renewal that moved
                            // the address or the router leaves only the new
                            // one once `converge` has applied it.
                            Ok(plan) => {
                                code = code.max(withdraw_stale(
                                    interface.name(),
                                    Some(&plan.address),
                                    plan.gateway,
                                ));
                                plans.push(plan);
                            }
                            // The server answered with something this machine
                            // cannot use. It is reported and NOT applied — a
                            // partially-applied lease is worse than none.
                            Err(why) => {
                                say(&render::tagged(&format!(
                                    "{} took a lease that cannot be applied: {why}",
                                    interface.name()
                                )));
                                code = code.max(1);
                            }
                        }
                    }
                    Err(why) => {
                        for line in render::lease_refused_lines(interface.name().as_str(), &why) {
                            say(&line);
                        }
                        code = code.max(1);
                    }
                }
            }
            Addressing::Static { .. } => match interface.static_plan() {
                Some(plan) => plans.push(plan),
                // Unreachable while `static_plan` returns `Some` for every
                // `Static` value, which it does by construction. It refuses
                // rather than skipping, because a skipped interface is an
                // unaddressed machine reported as a configured one.
                None => {
                    say(&render::tagged(&format!(
                        "{} is configured statically and no plan could be built for it; this is a \
                         defect in rivet-config, not in the document",
                        interface.name()
                    )));
                    code = code.max(1);
                }
            },
        }
    }

    if !plans.is_empty() {
        code = code.max(converge(&plans));
    }
    code
}

/// Join the wireless network an interface names.
///
/// # The order, and why each step is where it is
///
/// 1. **Raise the link.** The supplicant needs the interface up to scan.
/// 2. **Read the key**, before talking to anything — a machine with no key is
///    a machine that cannot join, and that is said before anything else is
///    attempted.
/// 3. **Reach the supplicant.** It is a service of its own,
///    `rivet_core::service::supplicant_service_name`, started by the
///    supervisor beside this program; this waits, bounded, for its control
///    socket rather than starting one.
/// 4. **Associate**, and wait for `COMPLETED` rather than for `ASSOCIATED`.
///
/// # Errors
///
/// A string naming what was wrong, for the caller to report. Every one of them
/// means the interface was NOT joined and is therefore not addressed either,
/// and the exit status that follows has the supervisor run this again.
#[cfg(target_os = "linux")]
fn join_wireless(
    interface: &rivet_core::net::InterfaceName,
    ssid: &rivet_core::ssid::Ssid,
) -> Result<(), String> {
    use rivet_adapter_net::{psk, supplicant};

    // The link first: a supplicant cannot scan on a down interface, and this is
    // the same step DHCP needs for the same reason.
    raise_link(interface)?;

    // **The key before the conversation.** A machine with no key says so
    // before anything is asked of the supplicant.
    let path = psk::path_of(interface);
    let stored = std::fs::read_to_string(&path).ok();
    let key = psk::read(stored, &path).map_err(|why| why.to_string())?;

    // **The supplicant's socket, waited for.** It is spawned in the same
    // convergence pass as this program and takes a moment to create it.
    let mut opened = None;
    for attempt in 0..SUPPLICANT_ATTEMPTS {
        if let Ok(control) = supplicant::ControlSocket::open(interface.as_str()) {
            opened = Some(control);
            break;
        }
        if attempt + 1 < SUPPLICANT_ATTEMPTS {
            std::thread::sleep(SUPPLICANT_INTERVAL);
        }
    }
    let mut control = opened.ok_or_else(|| {
        format!(
            "no supplicant answered on {}'s control socket under {}. It is the service {}; \
             `rivet get status` says whether it is running, and a card is built with it by \
             passing --supplicant to cmd/rivet-image",
            interface,
            supplicant::CONTROL_DIR,
            rivet_core::service::supplicant_service_name(interface.as_str())
        )
    })?;

    say(&render::tagged(&format!("{interface} joining {ssid}")));
    supplicant::associate(
        &mut control,
        ssid,
        // **The one place the key is exposed**, and it is named so that a
        // reviewer can find every one of them with a grep — `ADR-020` §4.
        // `associate` composes `SET_NETWORK <id> psk`, which is the wire form
        // and is where the bytes have to be in the clear.
        key.expose(),
        ASSOCIATE_ATTEMPTS,
        &mut || std::thread::sleep(ASSOCIATE_INTERVAL),
    )
    .map_err(|why| why.to_string())?;
    say(&render::tagged(&format!("{interface} joined {ssid}")));
    Ok(())
}

/// How many times to look for the supplicant's control socket: twenty looks,
/// a quarter of a second apart, for a process started in the same pass.
#[cfg(target_os = "linux")]
const SUPPLICANT_ATTEMPTS: u32 = 20;
/// How long between those looks.
#[cfg(target_os = "linux")]
const SUPPLICANT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
/// How long to wait for an association, in one-second steps. Scanning,
/// authenticating and the four-way handshake together take seconds on a quiet
/// band and longer on a busy one.
#[cfg(target_os = "linux")]
const ASSOCIATE_ATTEMPTS: u32 = 30;
/// The step between checks of `wpa_state`.
#[cfg(target_os = "linux")]
const ASSOCIATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Without netlink there is no joining anything, and saying so is the honest
/// answer — `AGENTS.md` §3.
#[cfg(not(target_os = "linux"))]
fn join_wireless(
    interface: &rivet_core::net::InterfaceName,
    _ssid: &rivet_core::ssid::Ssid,
) -> Result<(), String> {
    Err(format!(
        "this build has no netlink and no supplicant, so {interface} cannot join anything"
    ))
}

/// Bring one interface up, with no address, so a DHCP client can broadcast
/// and a supplicant can scan.
///
/// **It does not wait for a carrier.** The client is a supervised service
/// that keeps asking for as long as it runs, so a link whose carrier arrives
/// seconds later is answered at the client's next DISCOVER; nothing here has a
/// retry budget left to spend on a dead link. A wireless interface has no
/// carrier until it is associated, so waiting for one before associating
/// would only delay the join.
///
/// **Its own netlink socket, opened and closed around the one operation**, so
/// no borrow of `converge`'s socket spans the wireless join.
#[cfg(target_os = "linux")]
fn raise_link(interface: &rivet_core::net::InterfaceName) -> Result<(), String> {
    use rivet_adapter_net::link;
    use rivet_adapter_net::port::Sequence;

    let mut socket = rivet_adapter_net::socket::NetlinkSocket::open()
        .map_err(|errno| format!("cannot open a netlink socket: {errno}"))?;
    let mut seq = Sequence::new();
    match link::raise(&mut socket, &mut seq, interface) {
        Ok(true) => say(&render::tagged(&format!("{interface} raised"))),
        // Already up is the ordinary case on a second run and is not reported:
        // a line every restart saying nothing changed is noise in the one log
        // somebody reads when something did.
        Ok(false) => {}
        Err(failure) => return Err(failure.to_string()),
    }
    Ok(())
}

/// **Make a DHCP interface carry no IPv4 address but its lease's, and no
/// default route but one via its lease's router** — `ADR-004` §8, through
/// `rivet_adapter_net::wired::withdraw_all_but` and
/// `rivet_adapter_net::wired::withdraw_other_defaults`. `keep` is the lease's
/// address and `router` its router, each `None` when the lease names none.
///
/// Returns the exit code it contributes: 1 when the addresses or routes could
/// not be listed or one could not be removed, so the supervisor runs this
/// again. Its own netlink socket, opened and closed around the operation, as
/// [`raise_link`]'s.
#[cfg(target_os = "linux")]
fn withdraw_stale(
    interface: &rivet_core::net::InterfaceName,
    keep: Option<&rivet_core::net::Ipv4Cidr>,
    router: Option<std::net::Ipv4Addr>,
) -> u8 {
    let addresses = withdraw_stale_addresses(interface, keep);
    let routes = withdraw_stale_routes(interface, router);
    addresses.max(routes)
}

/// The default-route half of [`withdraw_stale`].
#[cfg(target_os = "linux")]
fn withdraw_stale_routes(
    interface: &rivet_core::net::InterfaceName,
    router: Option<std::net::Ipv4Addr>,
) -> u8 {
    let mut socket = match rivet_adapter_net::socket::NetlinkSocket::open() {
        Ok(socket) => socket,
        Err(errno) => {
            say(&render::tagged(&format!(
                "cannot open a netlink socket to read {interface}'s routes: {errno}"
            )));
            return 1;
        }
    };
    match rivet_adapter_net::wired::withdraw_other_defaults(&mut socket, interface, router) {
        Ok(withdrawn) => {
            for line in render::routes_withdrawn_lines(interface.as_str(), &withdrawn) {
                say(&line);
            }
            u8::from(render::any_route_refused(&withdrawn))
        }
        Err(why) => {
            say(&render::tagged(&format!(
                "could not read the routes leaving {interface}, so any default its lease does \
                 not name is still in place: {why}"
            )));
            1
        }
    }
}

/// The address half of [`withdraw_stale`].
#[cfg(target_os = "linux")]
fn withdraw_stale_addresses(
    interface: &rivet_core::net::InterfaceName,
    keep: Option<&rivet_core::net::Ipv4Cidr>,
) -> u8 {
    let mut socket = match rivet_adapter_net::socket::NetlinkSocket::open() {
        Ok(socket) => socket,
        Err(errno) => {
            say(&render::tagged(&format!(
                "cannot open a netlink socket to read {interface}'s addresses: {errno}"
            )));
            return 1;
        }
    };
    match rivet_adapter_net::wired::withdraw_all_but(&mut socket, interface, keep) {
        Ok(withdrawn) => {
            for line in render::withdrawn_lines(interface.as_str(), &withdrawn) {
                say(&line);
            }
            u8::from(render::any_refused(&withdrawn))
        }
        Err(why) => {
            say(&render::tagged(&format!(
                "could not read {interface}'s addresses, so any its lease does not name are \
                 still on it: {why}"
            )));
            1
        }
    }
}

/// Without netlink there is nothing to withdraw, and saying so is the honest
/// answer — `AGENTS.md` §3.
#[cfg(not(target_os = "linux"))]
fn withdraw_stale(
    interface: &rivet_core::net::InterfaceName,
    _keep: Option<&rivet_core::net::Ipv4Cidr>,
    _router: Option<std::net::Ipv4Addr>,
) -> u8 {
    say(&render::tagged(&format!(
        "this build has no netlink, so nothing on {interface} can be withdrawn"
    )));
    3
}

/// Without netlink there is nothing to raise, and saying so is the honest
/// answer — `AGENTS.md` §3.
#[cfg(not(target_os = "linux"))]
fn raise_link(interface: &rivet_core::net::InterfaceName) -> Result<(), String> {
    Err(format!(
        "this build has no netlink, so {interface} cannot be raised"
    ))
}

#[cfg(target_os = "linux")]
fn converge(plans: &[StaticAddressing]) -> u8 {
    use rivet_adapter_net::socket::NetlinkSocket;
    use rivet_adapter_net::wired;

    let mut socket = match NetlinkSocket::open() {
        Ok(socket) => socket,
        Err(errno) => {
            // Distinct from a step refusing: nothing was attempted, so nothing
            // is half-applied, and the cause is the kernel or the sandbox
            // rather than the plan.
            say(&render::tagged(&format!(
                "cannot open a netlink socket: {errno}"
            )));
            return 3;
        }
    };

    let mut code = 0_u8;
    for plan in plans {
        // Every interface is attempted. One that refuses does not skip the
        // rest — on a headless board the other link is the recovery path
        // (`ADR-004` §2.4), and abandoning it because the first failed would
        // remove the thing that makes the failure diagnosable.
        let report = wired::bring_up(&mut socket, plan);
        for line in render::report_lines(&report) {
            say(&line);
        }
        let step = render::exit_code(&report.health());
        code = code.max(u8::try_from(step).unwrap_or(1));
    }
    code
}

#[cfg(not(target_os = "linux"))]
fn converge(plans: &[StaticAddressing]) -> u8 {
    // `AGENTS.md` §3: a stub refuses. Returning 0 here would report a
    // configured network on a machine that has no netlink at all.
    for plan in plans {
        say(&render::tagged(&format!(
            "refusing to configure {} — netlink exists only on Linux, and this binary was built for {}",
            plan.interface,
            std::env::consts::OS
        )));
    }
    3
}

/// One console line.
///
/// Flushed rather than buffered: this runs during boot, and a machine that
/// wedges or is power-cycled must still have printed what it managed to do.
fn say(line: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use args::ArgError;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    /// A document in a temporary directory. `AGENTS.md` §8.3 — paths are
    /// arguments, never constants, and the suite writes what it needs rather
    /// than committing a fixture with a path in it.
    fn document_at(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rivet-netd-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join(name);
        std::fs::write(&path, text).expect("writing a fixture");
        path
    }

    #[test]
    fn help_exits_zero() {
        assert_eq!(run(&argv(&["--help"])), 0);
    }

    #[test]
    fn unusable_arguments_exit_two() {
        assert_eq!(run(&argv(&[])), 2);
        assert_eq!(run(&argv(&["--nonsense"])), 2);
        assert_eq!(run(&argv(&["--document"])), 2);
    }

    /// Every way `argv` can be wrong reaches exit 2 rather than one of them
    /// falling through to a different code.
    #[test]
    fn every_argument_error_exits_two() {
        for case in [vec!["--interface", "eth0"], vec!["--address", "10.0.0.1/24"], vec!["--dhcp"]] {
            assert_eq!(run(&argv(&case)), 2, "{case:?}");
        }
        let _exhaustive = |e: ArgError| match e {
            ArgError::Unknown(_) | ArgError::NeedsValue(_) | ArgError::Missing(_) => (),
        };
    }

    /// A document that is not there is the operator's mistake, not the
    /// machine's, and it must not be indistinguishable from an empty one.
    #[test]
    fn a_document_that_cannot_be_read_exits_two_rather_than_configuring_nothing() {
        let missing = std::env::temp_dir().join("rivet-netd-no-such-document.toml");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(
            run(&argv(&["--document", &missing.to_string_lossy()])),
            2
        );
    }

    #[test]
    fn a_document_that_is_refused_exits_two_rather_than_being_defaulted_around() {
        let path = document_at("refused.toml", "version = 1\nverison = 2\n");
        assert_eq!(run(&argv(&["--document", &path.to_string_lossy()])), 2);
    }

    /// The refusal that must never become a success. An interface asking for a
    /// lease exits non-zero and configures nothing, on **every** platform,
    /// because the mechanism it needs is not in the image (`ADR-004` §8).
    ///
    /// Note it is exit 1 rather than the old 2: the document is valid, and it
    /// is the *apply* that refused. Conflating the two would tell an operator
    /// their document was wrong.
    #[test]
    fn an_interface_asking_for_a_lease_refuses_at_apply_rather_than_at_parse() {
        let path = document_at("dhcp.toml", "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n");
        // It parses — that is the half `rivet-config` owns.
        let doc = load(&path).expect("a document asking for dhcp is a valid document");
        assert_eq!(doc.network().len(), 1);
        // ...and then apply refuses it.
        assert_eq!(run(&argv(&["--document", &path.to_string_lossy()])), 1);
    }

    /// A document naming no interface is not a failure. See the exit-code
    /// table: the empty configuration is applied, and a non-zero exit would
    /// have the supervisor restart this binary for ever.
    #[test]
    fn a_document_that_configures_no_interface_exits_zero() {
        for text in ["version = 1\n", "version = 1\n[network]\n"] {
            let path = document_at("empty-network.toml", text);
            assert_eq!(
                run(&argv(&["--document", &path.to_string_lossy()])),
                0,
                "{text:?}"
            );
        }
    }

    /// On the development host there is no netlink, and this binary says so
    /// rather than reporting a network it did not configure.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_non_linux_build_refuses_instead_of_claiming_success() {
        let path = document_at(
            "static.toml",
            "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n",
        );
        assert_eq!(run(&argv(&["--document", &path.to_string_lossy()])), 3);
    }

    /// The worst outcome wins. A document with a leased interface and a static
    /// one must not report success because the static half worked.
    ///
    /// **The interface names are deliberately ones no machine has.** This test
    /// runs on the developer's own Linux host under WSL, where `eth0` is a real
    /// interface carrying the session — `AGENTS.md` §9 says name the target
    /// explicitly rather than relying on an ambient check, and the target named
    /// here cannot be anything real. What the test asserts is the exit-code
    /// arithmetic, which does not need a live link.
    #[test]
    fn a_mixed_document_reports_the_worse_of_the_two() {
        let path = document_at(
            "mixed.toml",
            "version = 1\n\
             [network.rivet-t0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
             [network.rivet-t1]\nmethod = \"dhcp\"\n",
        );
        let code = run(&argv(&["--document", &path.to_string_lossy()]));
        assert_ne!(code, 0, "a refused lease must not report success");
    }
}
