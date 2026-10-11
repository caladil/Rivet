//! **Is there an adapter?** — asked of the kernel, not guessed at.
//!
//! `ADR-041` §6.4 answers R-144.1's *"what happens with no adapter"*, and this
//! module is the half that can be tested without one.
//!
//! # Why the management socket and not a probe of `hci0`
//!
//! Every Rivet built before `ADR-041` has no Bluetooth radio configured and the
//! x86 target may have no radio at all, so **no adapter is an ordinary state of
//! a machine** rather than a fault. The daemon has to say so and stay up.
//!
//! The cheap way to find out is to try to open `hci0` and read the errno. That
//! is `AGENTS.md` §7's failure exactly: `ENODEV` from a bind cannot distinguish
//! *there is no controller* from *the socket was refused for some other reason*,
//! and a daemon that concluded the first would report a fact about the machine
//! it had not established.
//!
//! `HCI_CHANNEL_CONTROL` answers positively instead. It binds with **zero**
//! controllers present — the 2026-09-05 study measured that, and read the
//! management interface's version back over it — so `Read Index List` returning
//! an empty list is the kernel *saying* there is nothing, and `Index Added`
//! arrives when that changes. No polling, no cached verdict, and the authority
//! for the answer is the kernel (`AGENTS.md` §5).
//!
//! # What this daemon does NOT do with this socket
//!
//! Everything else. `bluetoothd` drives the whole adapter through here —
//! discovery, pairing, connections, advertising. This daemon reads the index
//! list, listens for the two index events, and then hands the controller to a
//! user channel. **Binding a user channel removes the index**, so the events
//! below also record when this daemon itself took the adapter.

/// Every management packet: opcode, controller index, parameter length.
pub const HEADER_BYTES: usize = 6;

/// `MGMT_INDEX_NONE` — a command about the manager rather than a controller.
pub const INDEX_NONE: u16 = 0xFFFF;

/// `MGMT_OP_READ_INDEX_LIST`.
pub const OP_READ_INDEX_LIST: u16 = 0x0003;

/// `MGMT_EV_CMD_COMPLETE`.
pub const EV_COMMAND_COMPLETE: u16 = 0x0001;
/// `MGMT_EV_CMD_STATUS`.
pub const EV_COMMAND_STATUS: u16 = 0x0002;
/// `MGMT_EV_INDEX_ADDED`.
pub const EV_INDEX_ADDED: u16 = 0x0004;
/// `MGMT_EV_INDEX_REMOVED`.
pub const EV_INDEX_REMOVED: u16 = 0x0005;

/// The `Read Index List` request, as bytes.
#[must_use]
pub fn read_index_list() -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES);
    out.extend_from_slice(&OP_READ_INDEX_LIST.to_le_bytes());
    out.extend_from_slice(&INDEX_NONE.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// What arrived on the management socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// The reply to [`read_index_list`], already unpacked.
    IndexList { status: u8, indices: Vec<u16> },
    /// Some other command completed. Named rather than discarded so that a
    /// reply to something this daemon did not send is visible.
    OtherComplete { opcode: u16, status: u8 },
    /// A controller appeared.
    IndexAdded { index: u16 },
    /// A controller went away — unplugged, or **taken by a user channel**,
    /// including this daemon's own.
    IndexRemoved { index: u16 },
    /// Anything else the kernel volunteers.
    Other { event: u16, index: u16 },
}

/// Why a management packet could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketError {
    /// Fewer than [`HEADER_BYTES`] bytes.
    TooShort { len: usize },
    /// The header's length disagrees with what arrived.
    LengthMismatch { declared: usize, present: usize },
    /// A `Read Index List` reply whose parameters are shorter than the count it
    /// declares. Refused rather than truncated: a short list read as a complete
    /// one is a machine reporting fewer adapters than it has.
    ShortIndexList { declared: usize, present: usize },
}

impl core::fmt::Display for PacketError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { len } => write!(
                f,
                "{len} byte(s) arrived and a management packet header is {HEADER_BYTES}"
            ),
            Self::LengthMismatch { declared, present } => write!(
                f,
                "the management header declares {declared} parameter byte(s) and {present} arrived"
            ),
            Self::ShortIndexList { declared, present } => write!(
                f,
                "the controller index list declares {declared} entries and carries {present}"
            ),
        }
    }
}

impl core::error::Error for PacketError {}

/// Read one management packet.
///
/// # Errors
///
/// [`PacketError`].
pub fn parse(bytes: &[u8]) -> Result<Packet, PacketError> {
    if bytes.len() < HEADER_BYTES {
        return Err(PacketError::TooShort { len: bytes.len() });
    }
    let event = u16::from_le_bytes([bytes[0], bytes[1]]);
    let index = u16::from_le_bytes([bytes[2], bytes[3]]);
    let declared = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let params = &bytes[HEADER_BYTES..];
    if params.len() != declared {
        return Err(PacketError::LengthMismatch {
            declared,
            present: params.len(),
        });
    }
    Ok(match event {
        EV_COMMAND_COMPLETE => {
            // params: Command_Opcode (2), Status (1), Return_Parameters
            if params.len() < 3 {
                return Err(PacketError::LengthMismatch {
                    declared,
                    present: params.len(),
                });
            }
            let opcode = u16::from_le_bytes([params[0], params[1]]);
            let status = params[2];
            if opcode != OP_READ_INDEX_LIST {
                return Ok(Packet::OtherComplete { opcode, status });
            }
            // Return_Parameters: Num_Controllers (2), then that many indices.
            let rest = &params[3..];
            if status != 0x00 {
                return Ok(Packet::IndexList {
                    status,
                    indices: Vec::new(),
                });
            }
            if rest.len() < 2 {
                return Err(PacketError::ShortIndexList {
                    declared: 0,
                    present: rest.len(),
                });
            }
            let count = usize::from(u16::from_le_bytes([rest[0], rest[1]]));
            let body = &rest[2..];
            if body.len() < count * 2 {
                return Err(PacketError::ShortIndexList {
                    declared: count,
                    present: body.len() / 2,
                });
            }
            Packet::IndexList {
                status,
                indices: body[..count * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
                    .collect(),
            }
        }
        EV_COMMAND_STATUS if params.len() >= 3 => Packet::OtherComplete {
            opcode: u16::from_le_bytes([params[0], params[1]]),
            status: params[2],
        },
        EV_INDEX_ADDED => Packet::IndexAdded { index },
        EV_INDEX_REMOVED => Packet::IndexRemoved { index },
        other => Packet::Other {
            event: other,
            index,
        },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn complete(opcode: u16, status: u8, ret: &[u8]) -> Vec<u8> {
        let mut params = Vec::new();
        params.extend_from_slice(&opcode.to_le_bytes());
        params.push(status);
        params.extend_from_slice(ret);
        let mut out = Vec::new();
        out.extend_from_slice(&EV_COMMAND_COMPLETE.to_le_bytes());
        out.extend_from_slice(&INDEX_NONE.to_le_bytes());
        // A management parameter length is bounded by the kernel's own buffer;
        // every fixture here is a handful of bytes.
        out.extend_from_slice(&u16::try_from(params.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&params);
        out
    }

    #[test]
    fn the_request_is_six_bytes_naming_no_controller() {
        assert_eq!(read_index_list(), vec![0x03, 0x00, 0xff, 0xff, 0x00, 0x00]);
    }

    /// **The state every Rivet built before ADR-041 is in**, and it must be a
    /// reading rather than an error.
    #[test]
    fn a_machine_with_no_adapter_answers_an_empty_list_and_not_a_failure() {
        let bytes = complete(OP_READ_INDEX_LIST, 0x00, &[0x00, 0x00]);
        assert_eq!(
            parse(&bytes).unwrap(),
            Packet::IndexList {
                status: 0,
                indices: Vec::new()
            }
        );
    }

    #[test]
    fn two_controllers_are_both_read_in_the_order_the_kernel_gave_them() {
        let bytes = complete(
            OP_READ_INDEX_LIST,
            0x00,
            &[0x02, 0x00, 0x00, 0x00, 0x01, 0x00],
        );
        assert_eq!(
            parse(&bytes).unwrap(),
            Packet::IndexList {
                status: 0,
                indices: vec![0, 1]
            }
        );
    }

    /// A list that claims more entries than it carries must not be truncated
    /// into a shorter valid answer — that is a machine under-reporting its own
    /// hardware, which is the class `AGENTS.md` §5 refuses.
    #[test]
    fn a_short_index_list_is_refused_rather_than_truncated() {
        let bytes = complete(OP_READ_INDEX_LIST, 0x00, &[0x03, 0x00, 0x00, 0x00]);
        assert_eq!(
            parse(&bytes),
            Err(PacketError::ShortIndexList {
                declared: 3,
                present: 1
            })
        );
    }

    #[test]
    fn an_adapter_appearing_and_being_taken_are_both_reported() {
        let mut added = Vec::new();
        added.extend_from_slice(&EV_INDEX_ADDED.to_le_bytes());
        added.extend_from_slice(&0u16.to_le_bytes());
        added.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(parse(&added).unwrap(), Packet::IndexAdded { index: 0 });

        let mut removed = added.clone();
        removed[..2].copy_from_slice(&EV_INDEX_REMOVED.to_le_bytes());
        assert_eq!(parse(&removed).unwrap(), Packet::IndexRemoved { index: 0 });
    }

    #[test]
    fn a_reply_to_a_command_this_daemon_never_sent_is_named() {
        let bytes = complete(0x0009, 0x00, &[]);
        assert_eq!(
            parse(&bytes).unwrap(),
            Packet::OtherComplete {
                opcode: 0x0009,
                status: 0
            }
        );
    }

    #[test]
    fn a_short_or_mismatched_packet_is_refused() {
        assert_eq!(parse(&[0x01, 0x00]), Err(PacketError::TooShort { len: 2 }));
        assert_eq!(
            parse(&[0x04, 0x00, 0x00, 0x00, 0x04, 0x00]),
            Err(PacketError::LengthMismatch {
                declared: 4,
                present: 0
            })
        );
    }
}
