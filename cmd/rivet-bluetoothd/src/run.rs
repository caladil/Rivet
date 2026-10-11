//! **The loop**: find an adapter, take it, scan, forward.
//!
//! # It stays up with no adapter, and that is a decision
//!
//! R-144.1 item 4 — *"what happens with no adapter"* — and it is not a corner
//! case: **every Rivet built before `ADR-041` has no Bluetooth radio
//! configured**, and the x86 target named in the long-term plan may have none
//! at all. So the ordinary state of a machine running this daemon is that there
//! is nothing to scan with.
//!
//! `ADR-017` §3.5's policies are `always`, `on-failure` and `never`, and the
//! hazard §3.5 names is a restart loop writing to an SD card until the card
//! dies. A daemon that exited when it found no adapter would produce exactly
//! that under `always`, and under `on-failure` it would be marked `Failed`
//! after five attempts — a machine that could never notice a controller
//! appearing later.
//!
//! **So it does not exit. It blocks on the management socket and waits for
//! `Index Added`.** No polling, no backoff, no timer: the kernel tells it. That
//! is the shape `ADR-017` §3.2 asks of every service — *"an application that
//! needs something not yet up retries"* — with the retry being a blocking read
//! rather than a spin.
//!
//! The one thing it does exit for is a kernel with **no Bluetooth subsystem at
//! all**, because that is not a state that changes without a reboot into a
//! different image. It says so once, with the symbol name, and stops.
//!
//! # It gives up every capability once the controller is bound
//!
//! Binding `HCI_CHANNEL_USER` checks `CAP_NET_ADMIN`, and nothing after the
//! bind checks anything: the scan is commands written to, and events read from,
//! a socket this process already holds. So the moment the bind succeeds the
//! management socket is closed and [`crate::privilege::drop_all`] empties every
//! capability set. Every advertisement this daemon parses is parsed by a
//! process that holds nothing (`ADR-041` §6, `ADR-017` §3).
//!
//! # And it exits when it loses the controller
//!
//! A user channel is a claim on one controller. If that controller goes away —
//! a dongle unplugged, a driver unbound — the socket reads end, and a daemon
//! holding no capability cannot bind the next one. **It exits with
//! [`exit::CONTROLLER_GONE`]**, saying so, and the supervisor's `on-failure`
//! starts it again holding `CAP_NET_ADMIN`, back at the wait for `Index Added`.
//! A scan that lasted the supervisor's success window resets its give-up
//! count, so ordinary unplugging never ends a machine's Bluetooth. What it must
//! not do is hold a descriptor for a controller that is gone and report itself
//! healthy.

use crate::journal::{Journal, Severity, Value, field};
use crate::{exit, hci, mgmt, privilege, sockets, stream};
use rivet_core::bluetooth;

/// The most one HCI packet can be. An event's parameter length is one byte, so
/// 3 + 255 is the ceiling; the round number leaves the read obviously bounded.
const PACKET_MAX: usize = 512;

/// Run until stopped. The return value is the process's exit code.
pub fn run(journal: &mut Journal) -> i32 {
    let corr = journal.next_correlation();

    let listener = match sockets::handed_listener(bluetooth::ENDPOINT) {
        Ok(listener) => listener,
        Err(why) => {
            journal.emit(
                Severity::Error,
                &corr,
                "this service was not handed the socket its consumers connect to, so nothing \
                 can receive an advertisement and it was NOT started. The supervisor opens it \
                 from the definition's provides key (ADR-015 5.2, 5.3)",
                vec![
                    field("endpoint", Value::Str(bluetooth::ENDPOINT.to_string())),
                    field("detail", Value::Str(why)),
                ],
            );
            return exit::NO_SOCKET;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        journal.emit(
            Severity::Error,
            &corr,
            "the handed socket could not be made non-blocking, and an accept that waits would \
             stall the process that is reading the radio",
            vec![field("detail", Value::Str(e.to_string()))],
        );
        return exit::NO_SOCKET;
    }

    let management = match sockets::management() {
        Ok(socket) => socket,
        Err(e) => {
            journal.emit(
                Severity::Error,
                &corr,
                "this kernel has no Bluetooth subsystem, so there is nothing for this service \
                 to do and it stopped rather than looping. CONFIG_BT is what would change that, \
                 and it is a different image rather than a setting (ADR-024, ADR-041 5)",
                vec![
                    field("detail", Value::Str(e.to_string())),
                    field(
                        "consequence",
                        Value::Str(
                            "no application on this machine can read a BLE advertisement"
                                .to_string(),
                        ),
                    ),
                ],
            );
            return exit::NO_SUBSYSTEM;
        }
    };

    journal.emit(
        Severity::Info,
        &corr,
        "the management socket is bound, so this kernel HAS a Bluetooth subsystem. Asking it \
         which controllers exist",
        Vec::new(),
    );

    let mut consumers = stream::Consumers::new();
    let corr = journal.next_correlation();
    let Some(index) = wait_for_adapter(&management, &listener, &mut consumers, journal, &corr)
    else {
        return exit::STOPPED;
    };
    observe(index, management, &listener, &mut consumers, journal, &corr)
}

/// Block until the kernel says there is a controller, and return its index.
///
/// Connections are still accepted while waiting, so an application that starts
/// first is connected and simply hears nothing — which is the truthful state of
/// a board with no radio, and is better than a connection that is refused for a
/// reason the application cannot see.
fn wait_for_adapter(
    management: &sockets::Socket,
    listener: &std::os::unix::net::UnixListener,
    consumers: &mut stream::Consumers,
    journal: &mut Journal,
    corr: &str,
) -> Option<u16> {
    if let Err(e) = management.send(&mgmt::read_index_list()) {
        journal.emit(
            Severity::Error,
            corr,
            "the management socket would not take a Read Index List, so this daemon cannot find \
             out whether there is an adapter",
            vec![field("detail", Value::Str(e.to_string()))],
        );
        return None;
    }

    let mut said_none = false;
    let mut buf = [0u8; PACKET_MAX];
    loop {
        let fds = [management.fd(), listener_fd(listener)];
        let ready = match sockets::readable(&fds, None) {
            Ok(ready) => ready,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                journal.emit(
                    Severity::Error,
                    corr,
                    "poll refused while waiting for an adapter, so this daemon can no longer \
                     notice one appearing",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
                return None;
            }
        };
        if ready.get(1).copied().unwrap_or(false) {
            accept_waiting(listener, consumers, journal, corr);
        }
        if !ready.first().copied().unwrap_or(false) {
            continue;
        }
        let n = match management.recv(&mut buf) {
            Ok(0) => {
                journal.emit(
                    Severity::Error,
                    corr,
                    "the management socket closed, which nothing in this daemon asked for",
                    Vec::new(),
                );
                return None;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                journal.emit(
                    Severity::Error,
                    corr,
                    "the management socket refused a read",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
                return None;
            }
        };
        match mgmt::parse(&buf[..n]) {
            Ok(mgmt::Packet::IndexList { status, indices }) => {
                if status != 0 {
                    journal.emit(
                        Severity::Error,
                        corr,
                        "the kernel refused to list its Bluetooth controllers",
                        vec![field("status", Value::Int(i64::from(status)))],
                    );
                    return None;
                }
                if let Some(&index) = indices.first() {
                    return Some(index);
                }
                if !said_none {
                    said_none = true;
                    report_no_adapter(journal, corr);
                }
            }
            Ok(mgmt::Packet::IndexAdded { index }) => {
                journal.emit(
                    Severity::Info,
                    corr,
                    "a Bluetooth controller appeared",
                    vec![field("index", Value::Int(i64::from(index)))],
                );
                return Some(index);
            }
            // The kernel removing an index while this daemon is waiting for one
            // is ordinary: it is what a user channel binding looks like from
            // here, including this daemon's own.
            Ok(mgmt::Packet::IndexRemoved { .. } | mgmt::Packet::Other { .. }) => {}
            Ok(mgmt::Packet::OtherComplete { opcode, status }) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "the kernel completed a management command this daemon did not send, so \
                     something else is driving the management interface",
                    vec![
                        field("opcode", Value::Int(i64::from(opcode))),
                        field("status", Value::Int(i64::from(status))),
                    ],
                );
            }
            Err(e) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "a management packet could not be read and was discarded",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
            }
        }
    }
}

fn report_no_adapter(journal: &mut Journal, corr: &str) {
    journal.emit(
        Severity::Warn,
        corr,
        "this machine has NO Bluetooth controller. The kernel has the subsystem and reports \
         zero adapters, so no advertisement can arrive until one appears -- this service is \
         WAITING for that rather than failing, because a board with no radio is an ordinary \
         machine and a restart loop would not produce one (ADR-017 3.5)",
        vec![field(
            "next",
            Value::Str(
                "on a Raspberry Pi 4 the controller is the CYW43455 behind hci_uart; on a \
                 machine with none, a USB adapter would need CONFIG_BT_HCIBTUSB, which \
                 ADR-041 5.2 records as deliberately absent"
                    .to_string(),
            ),
        )],
    );
}

/// Take the adapter, give up every capability, and forward what it hears.
///
/// Returns the process's exit code: [`exit::CONTROLLER_GONE`] when the
/// controller cannot be taken, scanned or kept, [`exit::PRIVILEGE`] when the
/// drop is refused. The management socket is consumed, because nothing after
/// the bind needs it.
fn observe(
    index: u16,
    management: sockets::Socket,
    listener: &std::os::unix::net::UnixListener,
    consumers: &mut stream::Consumers,
    journal: &mut Journal,
    corr: &str,
) -> i32 {
    let adapter = match sockets::user_channel(index) {
        Ok(socket) => socket,
        Err(e) => {
            journal.emit(
                Severity::Error,
                corr,
                "this daemon could not take the Bluetooth controller, so it exits and the \
                 supervisor starts it again. It holds the adapter for the whole machine and only \
                 one process can (ADR-041 6.1), so an EUSERS here means something else already \
                 has it",
                vec![
                    field("index", Value::Int(i64::from(index))),
                    field("detail", Value::Str(e.to_string())),
                ],
            );
            return exit::CONTROLLER_GONE;
        }
    };
    drop(management);

    match privilege::drop_all() {
        Ok(held) => journal.emit(
            Severity::Info,
            corr,
            "the Bluetooth controller is bound, and this daemon gave up every capability it \
             held: nothing after the bind needs one, so every advertisement is parsed by a \
             process that holds nothing",
            vec![
                field("index", Value::Int(i64::from(index))),
                field("CapInh", Value::Str(privilege::mask(held.inheritable))),
                field("CapPrm", Value::Str(privilege::mask(held.permitted))),
                field("CapEff", Value::Str(privilege::mask(held.effective))),
                field("CapBnd", Value::Str(privilege::mask(held.bounding))),
                field("CapAmb", Value::Str(privilege::mask(held.ambient))),
                field("NoNewPrivs", Value::Int(i64::from(held.no_new_privs))),
            ],
        ),
        Err(e) => {
            journal.emit(
                Severity::Error,
                corr,
                "the Bluetooth controller is bound and this daemon could not give up every \
                 capability, so it stopped rather than scan holding what it should have dropped. \
                 Its definition must keep CAP_SETPCAP for the bounding set to be emptied \
                 (ADR-017 3)",
                vec![
                    field("index", Value::Int(i64::from(index))),
                    field("detail", Value::Str(e.to_string())),
                ],
            );
            return exit::PRIVILEGE;
        }
    }

    journal.emit(
        Severity::Info,
        corr,
        "the Bluetooth controller is held by this daemon alone, and the scan is being set up",
        vec![
            field("index", Value::Int(i64::from(index))),
            field("scan", Value::Str("passive".to_string())),
            field("interval_ms", Value::Int(interval_ms())),
            field("window_ms", Value::Int(window_ms())),
        ],
    );

    let mut sequence = hci::Sequence::new();
    let mut heard = 0u64;
    let mut buf = [0u8; PACKET_MAX];
    let mut announced = false;

    if !send_next(&adapter, &sequence, journal, corr) {
        return exit::CONTROLLER_GONE;
    }

    loop {
        let fds = [adapter.fd(), listener_fd(listener)];
        let ready = match sockets::readable(&fds, None) {
            Ok(ready) => ready,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                journal.emit(
                    Severity::Error,
                    corr,
                    "poll refused while scanning",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
                return exit::NO_SUBSYSTEM;
            }
        };
        if ready.get(1).copied().unwrap_or(false) {
            accept_waiting(listener, consumers, journal, corr);
        }
        if !ready.first().copied().unwrap_or(false) {
            continue;
        }
        let n = match adapter.recv(&mut buf) {
            Ok(0) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "the Bluetooth controller went away. This daemon holds no capability to bind \
                     another, so it exits and the supervisor starts it again to wait for one",
                    vec![
                        field("index", Value::Int(i64::from(index))),
                        field("advertisements", Value::Int(count_as_int(heard))),
                    ],
                );
                return exit::CONTROLLER_GONE;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "the Bluetooth controller refused a read, so this daemon exits and the \
                     supervisor starts it again to take the controller afresh",
                    vec![
                        field("detail", Value::Str(e.to_string())),
                        field("advertisements", Value::Int(count_as_int(heard))),
                    ],
                );
                return exit::CONTROLLER_GONE;
            }
        };
        let event = match hci::parse_event(&buf[..n]) {
            Ok(event) => event,
            Err(e) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "a packet from the controller could not be read and was discarded",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
                continue;
            }
        };

        if !announced {
            match sequence.accept(&event) {
                Ok(hci::Progress::Continue) => {
                    if !send_next(&adapter, &sequence, journal, corr) {
                        return exit::CONTROLLER_GONE;
                    }
                    continue;
                }
                Ok(hci::Progress::Scanning) => {
                    announced = true;
                    journal.emit(
                        Severity::Info,
                        corr,
                        "SCANNING. This machine is listening for BLE advertisements and every \
                         one it hears is handed to the application that declares connects on \
                         this service",
                        vec![field("index", Value::Int(i64::from(index)))],
                    );
                    continue;
                }
                Ok(hci::Progress::Ignored) => continue,
                Err(e) => {
                    journal.emit(
                        Severity::Error,
                        corr,
                        "the controller would not be put into scanning, so this machine hears \
                         nothing. This daemon exits and the supervisor starts it again",
                        vec![field("detail", Value::Str(e.to_string()))],
                    );
                    return exit::CONTROLLER_GONE;
                }
            }
        }

        let hci::Event::LeMeta { params } = event else {
            continue;
        };
        let at_ms = sockets::monotonic_ms();
        match bluetooth::parse_advertising_report(params, at_ms) {
            Ok(reports) => {
                for advertisement in reports {
                    heard = heard.saturating_add(1);
                    let mut line = advertisement.encode();
                    line.push('\n');
                    consumers.broadcast(&line);
                }
            }
            // Another LE subevent is not an error: the LE event mask asks for
            // one and a controller may volunteer others. Discarded silently
            // rather than journalled, because a mask that is being ignored
            // would otherwise fill the journal at the rate of the air.
            Err(bluetooth::ReportError::NotAnAdvertisingReport { .. }) => {}
            Err(e) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "an LE advertising report from the controller could not be read and was \
                     discarded",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
            }
        }
        for (at, dropped) in consumers.reports() {
            journal.emit(
                Severity::Warn,
                corr,
                "an application is not reading advertisements as fast as this machine hears \
                 them, so records were DROPPED rather than waited for. A daemon that blocked \
                 here would stop reading the radio for everybody",
                vec![
                    field("consumer", Value::Int(count_as_int(at as u64))),
                    field("dropped", Value::Int(count_as_int(dropped))),
                ],
            );
        }
    }
}

fn send_next(
    adapter: &sockets::Socket,
    sequence: &hci::Sequence,
    journal: &mut Journal,
    corr: &str,
) -> bool {
    let Some(command) = sequence.next_command() else {
        return true;
    };
    if let Err(e) = adapter.send(&command) {
        journal.emit(
            Severity::Error,
            corr,
            "the controller would not take a command, so the scan was never set up",
            vec![
                field(
                    "opcode",
                    Value::Int(sequence.outstanding().map_or(0, i64::from)),
                ),
                field("detail", Value::Str(e.to_string())),
            ],
        );
        return false;
    }
    true
}

fn accept_waiting(
    listener: &std::os::unix::net::UnixListener,
    consumers: &mut stream::Consumers,
    journal: &mut Journal,
    corr: &str,
) {
    loop {
        match listener.accept() {
            Ok((socket, _)) => match stream::Consumer::new(socket) {
                Ok(consumer) => {
                    if consumers.push(consumer) {
                        journal.emit(
                            Severity::Info,
                            corr,
                            "an application connected and will receive every advertisement this \
                             machine hears",
                            vec![field("connections", Value::Int(count_as_int(
                                consumers.len() as u64,
                            )))],
                        );
                    } else {
                        journal.emit(
                            Severity::Warn,
                            corr,
                            "a connection was refused because this daemon already holds as many \
                             as it will. An endpoint has one consumer (ADR-015 5.3); a client \
                             reconnecting in a loop is what reaches this",
                            vec![field(
                                "refused",
                                Value::Int(count_as_int(consumers.refused())),
                            )],
                        );
                    }
                }
                Err(e) => journal.emit(
                    Severity::Warn,
                    corr,
                    "a connection could not be made non-blocking and was dropped",
                    vec![field("detail", Value::Str(e.to_string()))],
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                journal.emit(
                    Severity::Warn,
                    corr,
                    "the handed socket refused an accept",
                    vec![field("detail", Value::Str(e.to_string()))],
                );
                return;
            }
        }
    }
}

fn listener_fd(listener: &std::os::unix::net::UnixListener) -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd as _;
    listener.as_raw_fd()
}

/// A count as a journal integer, saturating rather than wrapping.
///
/// `ADR-022` §3.2's `Value::Int` is signed. A count that exceeded it would be a
/// number no board will reach, and saturating says so rather than reporting a
/// negative one.
fn count_as_int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The scan interval in milliseconds, for the journal. The wire units are
/// 0.625 ms (`hci::SCAN_INTERVAL`), and a record saying `160` would be a record
/// nobody can read.
fn interval_ms() -> i64 {
    i64::from(hci::SCAN_INTERVAL) * 5 / 8
}

/// The scan window in milliseconds. See [`interval_ms`].
fn window_ms() -> i64 {
    i64::from(hci::SCAN_WINDOW) * 5 / 8
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The journal reports milliseconds, not 0.625 ms units. A reader meeting
    /// `interval 160` would have no way to know it was not 160 ms.
    #[test]
    fn the_journal_reports_the_duty_cycle_in_milliseconds() {
        assert_eq!(interval_ms(), 100);
        assert_eq!(window_ms(), 30);
    }

    /// **Losing the controller is a failure, so `on-failure` brings the daemon
    /// back**; a clean exit there would leave the machine's Bluetooth `done`.
    /// Every exit says something different, so no two share a number.
    #[test]
    fn losing_the_controller_is_an_exit_the_supervisor_restarts() {
        assert_ne!(exit::CONTROLLER_GONE, exit::STOPPED);
        assert_ne!(exit::PRIVILEGE, exit::STOPPED);
        let mut codes = [
            exit::STOPPED,
            exit::NO_SOCKET,
            exit::NO_SUBSYSTEM,
            exit::CONTROLLER_GONE,
            exit::PRIVILEGE,
        ];
        codes.sort_unstable();
        let before = codes.len();
        let mut distinct = codes.to_vec();
        distinct.dedup();
        assert_eq!(distinct.len(), before, "{codes:?}");
    }

    #[test]
    fn a_count_that_cannot_fit_saturates_rather_than_going_negative() {
        assert_eq!(count_as_int(7), 7);
        assert_eq!(count_as_int(u64::MAX), i64::MAX);
    }
}
