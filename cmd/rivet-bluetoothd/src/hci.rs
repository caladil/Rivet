//! **The five commands that turn a controller into an observer**, and the
//! events that come back.
//!
//! `ADR-041` §4. Everything in this file is bytes in and bytes out: no socket,
//! no descriptor, no `libc`. That is deliberate and it is `AGENTS.md` §3.1's
//! second kind of seam — an adapter hiding a platform from its own tests. The
//! socket is [`crate::sockets`]; the sequence below is provable without one,
//! and `cargo test -p rivetos-bluetoothd` runs it on any host.
//!
//! # Why five commands and not a stack
//!
//! Reading advertisements is the whole ruled capability (`.rulings/` R-144).
//! Reset the controller, say which events may come back, say which LE events
//! may come back, set the scan parameters, turn the scan on. After that the
//! controller sends `LE Advertising Report` and this daemon forwards it.
//!
//! **There is deliberately no connection, no bonding and no GATT.** R-144.1
//! item 4: a daemon built ready for tier 2 carries untested surface for a
//! capability nobody ruled.

/// The HCI packet type that prefixes a command on a user-channel socket.
pub const PKT_COMMAND: u8 = 0x01;
/// The HCI packet type that prefixes an event.
pub const PKT_EVENT: u8 = 0x04;

/// `HCI_Reset`. OGF 0x03, OCF 0x0003.
pub const OP_RESET: u16 = 0x0C03;
/// `HCI_Set_Event_Mask`. OGF 0x03, OCF 0x0001.
pub const OP_SET_EVENT_MASK: u16 = 0x0C01;
/// `HCI_LE_Set_Event_Mask`. OGF 0x08, OCF 0x0001.
pub const OP_LE_SET_EVENT_MASK: u16 = 0x2001;
/// `HCI_LE_Set_Scan_Parameters`. OGF 0x08, OCF 0x000B.
pub const OP_LE_SET_SCAN_PARAMETERS: u16 = 0x200B;
/// `HCI_LE_Set_Scan_Enable`. OGF 0x08, OCF 0x000C.
pub const OP_LE_SET_SCAN_ENABLE: u16 = 0x200C;

/// `HCI_Command_Complete`.
pub const EV_COMMAND_COMPLETE: u8 = 0x0E;
/// `HCI_Command_Status`.
pub const EV_COMMAND_STATUS: u8 = 0x0F;

/// The event mask this daemon sets, little-endian on the wire.
///
/// **The specification's own default, plus bit 61.** After `HCI_Reset` the mask
/// is `0x0000_1FFF_FFFF_FFFF`, which does **not** include `LE Meta Event` — so
/// a controller that had been told nothing else would scan correctly and report
/// nothing, which is the quietest possible failure. Bit 61 is `LE Meta Event`.
///
/// Leaving the rest of the default alone rather than narrowing it is deliberate:
/// the events it admits are for a stack that connects, this daemon does not, and
/// a controller with nothing to say about connections says nothing about them.
pub const EVENT_MASK: u64 = 0x2000_1FFF_FFFF_FFFF;

/// The LE event mask, and it is one bit.
///
/// Bit 1 is `LE Advertising Report`. **Everything else is off**, including
/// `LE Connection Complete` and `LE Enhanced Connection Complete` — this daemon
/// cannot connect, so an event about a connection would be an event about
/// something that did not happen. The specification's default here is
/// `0x0000_0000_0000_001F`, five bits, four of which are about connections.
pub const LE_EVENT_MASK: u64 = 0x0000_0000_0000_0002;

/// Scan interval, in units of 0.625 ms. `0x00A0` is 100 ms.
///
/// # This pair is a POWER decision and it has not been measured
///
/// R-144.1 item 5 is explicit that it cannot be, here or in any emulator: LE
/// scanning and the wireless client share one Broadcom part, and what a duty
/// cycle costs a Rivet is a question for a board with a meter on it.
///
/// So the choice is made on the side that can be argued from the air interface
/// rather than from a guess about the radio. [`SCAN_WINDOW`] is 30 ms in this
/// 100 ms interval — the receiver is **off 70% of the time** — and a sensor
/// broadcasting every two seconds repeats each advertising event on all three
/// primary channels, so a 30% duty cycle hears one within a few seconds rather
/// than within one interval. Continuous scanning (window equal to interval) is
/// what a tool like `btmon` does and is the wrong default for a board that is
/// also holding a wireless association.
///
/// **This is the knob to make configurable the day there is a measurement**,
/// and `ADR-041` §6 says why it is a constant until then: a schema key with an
/// unmeasured value behind it invites an operator to tune something nobody can
/// tell them the cost of.
pub const SCAN_INTERVAL: u16 = 0x00A0;

/// Scan window, in units of 0.625 ms. `0x0030` is 30 ms. See [`SCAN_INTERVAL`].
pub const SCAN_WINDOW: u16 = 0x0030;

/// **The duty cycle is a duty cycle, checked by the compiler.**
///
/// A window equal to the interval is continuous reception, which is the power
/// posture `ADR-041` §6 declines to take without a measurement — and the
/// measurement cannot be taken in an emulator, because the radio it would cost
/// power on is not there. Two edits could remove the gap silently: raising the
/// window, or lowering the interval.
///
/// It was a `#[test]` until 2026-09-05, comparing two constants at run time,
/// which clippy correctly called an assertion with a constant value. A property
/// no input can influence belongs to the build.
const _A_DUTY_CYCLE_IS_NOT_CONTINUOUS: () = assert!(
    SCAN_WINDOW < SCAN_INTERVAL,
    "the scan window must be shorter than the interval; equal is continuous reception"
);

/// Passive scanning: the receiver listens and never transmits.
///
/// **Active scanning would send `SCAN_REQ` and collect scan responses**, which
/// is how a device's name is usually learned. Tier 1 does not need it: the
/// maintainer's sensors put their reading in the advertisement itself, so
/// passive is both sufficient and the half that does not power a transmitter.
/// A daemon that transmitted would also be announcing this board's presence to
/// every device in the house, which is not what *reading advertisements* means.
pub const SCAN_TYPE_PASSIVE: u8 = 0x00;

/// Report every advertisement; do not let the controller suppress repeats.
///
/// **This is the one parameter where the cheap answer is wrong.** The
/// controller's duplicate filter exists precisely for a scanner drowning in
/// traffic, and the Bluetooth Core specification permits it to be implemented
/// on the advertiser's **address** alone. A sensor's whole purpose is that its
/// payload changes while its address does not — so a controller filtering on
/// address would deliver the first reading and silently drop every later one,
/// which is exactly the ruled capability failing in the quietest possible way.
///
/// The cost is volume, and `crate::stream` is where it is bounded: a consumer
/// that cannot keep up has records **dropped and counted**, never blocked on.
pub const FILTER_DUPLICATES_OFF: u8 = 0x00;

/// Compose an HCI command packet, ready to write to a user-channel socket.
#[must_use]
pub fn command(opcode: u16, params: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + params.len());
    out.push(PKT_COMMAND);
    out.extend_from_slice(&opcode.to_le_bytes());
    // A command's parameter total length is one byte, and every command in this
    // file is under ten. `u8::try_from` rather than `as` so that a future one
    // that is not cannot silently wrap.
    out.push(u8::try_from(params.len()).unwrap_or(u8::MAX));
    out.extend_from_slice(params);
    out
}

/// What arrived from the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// A command finished. `status` is the first parameter byte for every
    /// command this daemon sends, and `None` when the controller returned none
    /// — which is a malformed reply rather than a success.
    CommandComplete { opcode: u16, status: Option<u8> },
    /// A command was accepted and will finish later. None of the five commands
    /// here answers this way; it is parsed so that one arriving is *named*
    /// rather than falling into [`Event::Other`].
    CommandStatus { opcode: u16, status: u8 },
    /// An `LE Meta Event`. `params` begins with the subevent code, which is what
    /// `rivet_core::bluetooth::parse_advertising_report` expects.
    LeMeta { params: &'a [u8] },
    /// Anything else. Carried rather than discarded so the journal can say what
    /// a controller sent that was not asked for.
    Other { code: u8 },
}

/// Why a packet from the controller could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventError {
    /// Fewer than the three bytes an HCI event header needs.
    TooShort { len: usize },
    /// The first byte is not [`PKT_EVENT`]. An ACL or SCO packet on a socket
    /// that asked for neither lands here.
    NotAnEvent { packet_type: u8 },
    /// The header's parameter length disagrees with how many bytes arrived.
    LengthMismatch { declared: usize, present: usize },
}

impl core::fmt::Display for EventError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { len } => write!(
                f,
                "{len} byte(s) arrived and an HCI event header is three: a packet type, an event \
                 code and a parameter length"
            ),
            Self::NotAnEvent { packet_type } => write!(
                f,
                "packet type 0x{packet_type:02x} on the user channel, and this daemon reads only \
                 events (0x04). It sends no ACL and opens no SCO"
            ),
            Self::LengthMismatch { declared, present } => write!(
                f,
                "the event header declares {declared} parameter byte(s) and {present} arrived"
            ),
        }
    }
}

impl core::error::Error for EventError {}

/// Read one packet from a user-channel socket.
///
/// # Errors
///
/// [`EventError`], which names which of the three ways the packet was wrong.
pub fn parse_event(packet: &[u8]) -> Result<Event<'_>, EventError> {
    if packet.len() < 3 {
        return Err(EventError::TooShort { len: packet.len() });
    }
    if packet[0] != PKT_EVENT {
        return Err(EventError::NotAnEvent {
            packet_type: packet[0],
        });
    }
    let code = packet[1];
    let declared = usize::from(packet[2]);
    let params = &packet[3..];
    if params.len() != declared {
        return Err(EventError::LengthMismatch {
            declared,
            present: params.len(),
        });
    }
    Ok(match code {
        EV_COMMAND_COMPLETE if params.len() >= 3 => Event::CommandComplete {
            // params: Num_HCI_Command_Packets, Opcode (2), Return_Parameters
            opcode: u16::from_le_bytes([params[1], params[2]]),
            status: params.get(3).copied(),
        },
        EV_COMMAND_STATUS if params.len() >= 4 => Event::CommandStatus {
            // params: Status, Num_HCI_Command_Packets, Opcode (2)
            opcode: u16::from_le_bytes([params[2], params[3]]),
            status: params[0],
        },
        rivet_core::bluetooth::HCI_EV_LE_META => Event::LeMeta { params },
        other => Event::Other { code: other },
    })
}

/// The five commands, in order, and where the daemon is in them.
///
/// # Why a state machine and not five blocking round trips
///
/// Because a blocking round trip cannot be tested without a controller, and
/// because the thing most likely to be wrong here is the **order** and the
/// **matching of a reply to the command that asked for it** — a sequence that
/// accepted any `Command Complete` would advance on the wrong one and turn on a
/// scan that had never been configured. That failure succeeds; it does not
/// error. So the matching is a pure function with a test on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sequence {
    at: usize,
}

/// One step of [`Sequence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The reply matched; send [`Sequence::next_command`].
    Continue,
    /// Every command has been acknowledged. The controller is scanning.
    Scanning,
    /// A reply arrived that this step was not waiting for, and it was ignored
    /// rather than treated as progress. Carries the code so the journal can say
    /// what the controller volunteered.
    Ignored,
}

/// Why the setup sequence stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    /// The controller refused a command. `status` is its own error code.
    Refused { opcode: u16, status: u8 },
    /// A `Command Complete` with no status byte in it.
    NoStatus { opcode: u16 },
    /// A `Command Complete` for a command that was not the one outstanding.
    /// **Not ignored**: the controller is answering something this daemon did
    /// not ask, which means it is not the controller this daemon thinks it is.
    WrongCommand { expected: u16, got: u16 },
}

impl core::fmt::Display for SetupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused { opcode, status } => write!(
                f,
                "the controller refused HCI opcode 0x{opcode:04x} with status 0x{status:02x}, so \
                 the adapter was never put into scanning and no advertisement can arrive"
            ),
            Self::NoStatus { opcode } => write!(
                f,
                "the controller completed HCI opcode 0x{opcode:04x} with no status byte, which is \
                 not a reply this command has"
            ),
            Self::WrongCommand { expected, got } => write!(
                f,
                "the controller completed HCI opcode 0x{got:04x} while 0x{expected:04x} was \
                 outstanding. Nothing else sent it a command, so this adapter is being driven by \
                 something other than this daemon"
            ),
        }
    }
}

impl core::error::Error for SetupError {}

/// The opcodes, in the order they are sent.
const ORDER: [u16; 5] = [
    OP_RESET,
    OP_SET_EVENT_MASK,
    OP_LE_SET_EVENT_MASK,
    OP_LE_SET_SCAN_PARAMETERS,
    OP_LE_SET_SCAN_ENABLE,
];

impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequence {
    #[must_use]
    pub const fn new() -> Self {
        Self { at: 0 }
    }

    /// The command to send now, or `None` when the controller is scanning.
    #[must_use]
    pub fn next_command(&self) -> Option<Vec<u8>> {
        Some(match *ORDER.get(self.at)? {
            OP_RESET => command(OP_RESET, &[]),
            OP_SET_EVENT_MASK => command(OP_SET_EVENT_MASK, &EVENT_MASK.to_le_bytes()),
            OP_LE_SET_EVENT_MASK => command(OP_LE_SET_EVENT_MASK, &LE_EVENT_MASK.to_le_bytes()),
            OP_LE_SET_SCAN_PARAMETERS => {
                let mut p = Vec::with_capacity(7);
                p.push(SCAN_TYPE_PASSIVE);
                p.extend_from_slice(&SCAN_INTERVAL.to_le_bytes());
                p.extend_from_slice(&SCAN_WINDOW.to_le_bytes());
                // Own_Address_Type 0x00 — this board's public address. A random
                // one would need the controller to have been given one, which
                // is a command this daemon does not send.
                p.push(0x00);
                // Scanning_Filter_Policy 0x00 — accept all. A white list is a
                // list this daemon has no way to be told (`ADR-041` §6).
                p.push(0x00);
                command(OP_LE_SET_SCAN_PARAMETERS, &p)
            }
            OP_LE_SET_SCAN_ENABLE => {
                command(OP_LE_SET_SCAN_ENABLE, &[0x01, FILTER_DUPLICATES_OFF])
            }
            // `ORDER` is a const array and every arm above covers one of its
            // five entries, so this is unreachable rather than a case.
            other => command(other, &[]),
        })
    }

    /// The opcode this sequence is waiting on, or `None` when it is scanning.
    #[must_use]
    pub fn outstanding(&self) -> Option<u16> {
        ORDER.get(self.at).copied()
    }

    /// Feed one event.
    ///
    /// # Errors
    ///
    /// [`SetupError`] when the controller refused the outstanding command,
    /// answered a different one, or answered with no status.
    pub fn accept(&mut self, event: &Event<'_>) -> Result<Progress, SetupError> {
        let Some(expected) = self.outstanding() else {
            return Ok(Progress::Scanning);
        };
        match event {
            Event::CommandComplete { opcode, status } => {
                if *opcode != expected {
                    return Err(SetupError::WrongCommand {
                        expected,
                        got: *opcode,
                    });
                }
                let status = status.ok_or(SetupError::NoStatus { opcode: *opcode })?;
                if status != 0x00 {
                    return Err(SetupError::Refused {
                        opcode: *opcode,
                        status,
                    });
                }
                self.at += 1;
                Ok(if self.at == ORDER.len() {
                    Progress::Scanning
                } else {
                    Progress::Continue
                })
            }
            Event::CommandStatus { opcode, status } if *opcode == expected && *status != 0 => {
                Err(SetupError::Refused {
                    opcode: *opcode,
                    status: *status,
                })
            }
            // An advertising report before the scan was enabled, or a vendor
            // event a controller volunteers at reset. Neither is progress and
            // neither is a fault.
            _ => Ok(Progress::Ignored),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The controller's side of a `Command Complete`, as a byte packet.
    fn complete(opcode: u16, status: u8) -> Vec<u8> {
        let op = opcode.to_le_bytes();
        vec![PKT_EVENT, EV_COMMAND_COMPLETE, 0x04, 0x01, op[0], op[1], status]
    }

    #[test]
    fn a_command_is_a_type_byte_an_opcode_and_a_length() {
        assert_eq!(command(OP_RESET, &[]), vec![0x01, 0x03, 0x0c, 0x00]);
        assert_eq!(
            command(OP_LE_SET_SCAN_ENABLE, &[0x01, 0x00]),
            vec![0x01, 0x0c, 0x20, 0x02, 0x01, 0x00]
        );
    }

    /// The exact bytes the study measured a controller receiving, so a change
    /// to the opcode composition is visible rather than merely re-derived.
    #[test]
    fn the_reset_command_is_the_four_bytes_the_2026_09_05_study_measured() {
        assert_eq!(command(OP_RESET, &[]), vec![0x01, 0x03, 0x0c, 0x00]);
    }

    #[test]
    fn the_five_commands_are_sent_in_order_and_the_last_one_starts_the_scan() {
        let mut seq = Sequence::new();
        let mut sent = Vec::new();
        while let Some(cmd) = seq.next_command() {
            let opcode = u16::from_le_bytes([cmd[1], cmd[2]]);
            sent.push(opcode);
            let packet = complete(opcode, 0x00);
            let ev = parse_event(&packet).unwrap();
            match seq.accept(&ev).unwrap() {
                Progress::Continue => {}
                Progress::Scanning => break,
                Progress::Ignored => panic!("a matching completion was ignored"),
            }
        }
        assert_eq!(sent, ORDER.to_vec());
        assert_eq!(seq.outstanding(), None);
        assert_eq!(seq.next_command(), None);
    }

    /// **The failure this guards succeeds rather than errors.** A sequence that
    /// accepted any completion would advance on a reply to a command it never
    /// sent and enable a scan that was never configured.
    #[test]
    fn a_completion_for_another_command_is_refused_and_does_not_advance() {
        let mut seq = Sequence::new();
        let packet = complete(OP_LE_SET_SCAN_ENABLE, 0x00);
        let ev = parse_event(&packet).unwrap();
        assert_eq!(
            seq.accept(&ev),
            Err(SetupError::WrongCommand {
                expected: OP_RESET,
                got: OP_LE_SET_SCAN_ENABLE
            })
        );
        assert_eq!(seq.outstanding(), Some(OP_RESET));
    }

    #[test]
    fn a_refusal_names_the_opcode_and_the_status() {
        let mut seq = Sequence::new();
        let packet = complete(OP_RESET, 0x0C);
        let ev = parse_event(&packet).unwrap();
        let err = seq.accept(&ev).unwrap_err();
        assert_eq!(
            err,
            SetupError::Refused {
                opcode: OP_RESET,
                status: 0x0C
            }
        );
        assert!(err.to_string().contains("0x0c03"));
    }

    #[test]
    fn an_event_arriving_mid_sequence_is_ignored_rather_than_counted() {
        let mut seq = Sequence::new();
        let vendor = vec![PKT_EVENT, 0xFF, 0x01, 0x2A];
        let ev = parse_event(&vendor).unwrap();
        assert_eq!(seq.accept(&ev), Ok(Progress::Ignored));
        assert_eq!(seq.outstanding(), Some(OP_RESET));
    }

    #[test]
    fn an_le_meta_event_carries_the_subevent_code_first() {
        let packet = vec![PKT_EVENT, 0x3E, 0x03, 0x02, 0x00, 0x00];
        match parse_event(&packet).unwrap() {
            Event::LeMeta { params } => assert_eq!(params, &[0x02, 0x00, 0x00]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_packet_that_is_not_an_event_is_named_rather_than_read() {
        assert_eq!(
            parse_event(&[0x02, 0x00, 0x00]),
            Err(EventError::NotAnEvent { packet_type: 0x02 })
        );
        assert_eq!(parse_event(&[0x04, 0x0e]), Err(EventError::TooShort { len: 2 }));
        assert_eq!(
            parse_event(&[0x04, 0x0e, 0x04, 0x01]),
            Err(EventError::LengthMismatch {
                declared: 4,
                present: 1
            })
        );
    }

    /// The mask is what makes an advertising report reach userspace at all, and
    /// getting it wrong produces a scanner that works and reports nothing.
    #[test]
    fn the_event_mask_admits_le_meta_and_the_le_mask_admits_only_the_report() {
        assert_eq!(EVENT_MASK & (1 << 61), 1 << 61);
        assert_eq!(LE_EVENT_MASK, 1 << 1);
        let cmd = Sequence { at: 1 }.next_command().unwrap();
        assert_eq!(&cmd[4..], &EVENT_MASK.to_le_bytes());
        let cmd = Sequence { at: 2 }.next_command().unwrap();
        assert_eq!(&cmd[4..], &LE_EVENT_MASK.to_le_bytes());
    }

    #[test]
    fn the_scan_is_passive_duty_cycled_and_does_not_filter_duplicates() {
        let params = Sequence { at: 3 }.next_command().unwrap();
        assert_eq!(params[4], SCAN_TYPE_PASSIVE);
        assert_eq!(u16::from_le_bytes([params[5], params[6]]), SCAN_INTERVAL);
        assert_eq!(u16::from_le_bytes([params[7], params[8]]), SCAN_WINDOW);
        // The window-against-interval property used to be asserted here, where
        // it compared two constants at run time and clippy correctly called it
        // a constant assertion. It is now `_A_DUTY_CYCLE_IS_NOT_CONTINUOUS`
        // below, which fails the BUILD rather than a test — the stronger place
        // for a property that no input can influence.
        let enable = Sequence { at: 4 }.next_command().unwrap();
        assert_eq!(enable[4], 0x01);
        assert_eq!(enable[5], FILTER_DUPLICATES_OFF);
    }
}
