//! **The kernel's CEC interface, restated** — `linux/cec.h`.
//!
//! # Every number here was PRINTED by the header, not derived from it
//!
//! An ioctl request encodes the direction and the **size of the struct** it
//! carries; a layout that disagrees with the kernel's by one byte produces a
//! request number the kernel does not recognise, and `ioctl` answers `ENOTTY` —
//! which reads as *this device does not support that*, not as *your struct is
//! wrong*. That is a whole evening.
//!
//! So they were not worked out. A C program was compiled against
//! `/br/host/aarch64-buildroot-linux-musl/sysroot/usr/include/linux/cec.h` —
//! the header for the kernel the board is running — and made to print every
//! constant, every `sizeof` and every `offsetof` below. `tests` asserts the
//! Rust matches what it printed, so a future header that moves a field fails
//! here rather than on a television.
//!
//! **The layout is the same on both architectures**, which is why that C
//! program could be compiled for x86_64 and still be authoritative: every field
//! in these three structs is a fixed-width type. No pointer, no `long`, no
//! `size_t`.

#![allow(dead_code)] // the whole uapi is restated; not all of it is called yet

/// `CEC_MAX_MSG_SIZE`.
pub const MAX_MSG_SIZE: usize = 16;
/// `CEC_MAX_LOG_ADDRS`.
pub const MAX_LOG_ADDRS: usize = 4;

// --- the ioctl requests, printed from the header ---------------------------
//
// `_IOWR('a', N, struct ...)`, and the struct size is baked into the number.
pub const ADAP_G_CAPS: u32 = 0xc04c_6100;
pub const ADAP_G_PHYS_ADDR: u32 = 0x8002_6101;
pub const ADAP_S_PHYS_ADDR: u32 = 0x4002_6102;
pub const ADAP_G_LOG_ADDRS: u32 = 0x805c_6103;
pub const ADAP_S_LOG_ADDRS: u32 = 0xc05c_6104;
pub const TRANSMIT: u32 = 0xc038_6105;
pub const S_MODE: u32 = 0x4004_6109;

// --- capabilities ----------------------------------------------------------
pub const CAP_PHYS_ADDR: u32 = 0x0000_0001;
pub const CAP_LOG_ADDRS: u32 = 0x0000_0002;
pub const CAP_TRANSMIT: u32 = 0x0000_0004;

// --- modes -----------------------------------------------------------------
pub const MODE_INITIATOR: u32 = 0x0000_0001;
pub const MODE_EXCL_INITIATOR: u32 = 0x0000_0002;

// --- logical addresses -----------------------------------------------------
/// The television is always logical address 0. That is the protocol, not a
/// convention: `CEC_LOG_ADDR_TV`.
pub const LOG_ADDR_TV: u8 = 0;
/// `CEC_LOG_ADDR_UNREGISTERED` — what an adapter that has claimed nothing is.
pub const LOG_ADDR_UNREGISTERED: u8 = 15;
/// `CEC_LOG_ADDR_TYPE_PLAYBACK`.
pub const LOG_ADDR_TYPE_PLAYBACK: u8 = 3;
/// `CEC_OP_PRIM_DEVTYPE_PLAYBACK`.
pub const PRIM_DEVTYPE_PLAYBACK: u8 = 4;
/// `CEC_OP_CEC_VERSION_2_0`.
pub const VERSION_2_0: u8 = 6;
/// `CEC_VENDOR_ID_NONE`.
pub const VENDOR_ID_NONE: u32 = 0xffff_ffff;
/// `CEC_PHYS_ADDR_INVALID` — no cable, or no EDID to read one from.
pub const PHYS_ADDR_INVALID: u16 = 0xffff;

// --- opcodes ---------------------------------------------------------------
/// `CEC_MSG_STANDBY` — **the one that saves the power.**
pub const MSG_STANDBY: u8 = 0x36;
/// `CEC_MSG_IMAGE_VIEW_ON` — wake, and switch to this input.
pub const MSG_IMAGE_VIEW_ON: u8 = 0x04;

/// `CEC_TX_STATUS_OK`.
pub const TX_STATUS_OK: u8 = 0x01;

/// `struct cec_msg` — 56 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Msg {
    pub tx_ts: u64,
    pub rx_ts: u64,
    pub len: u32,
    pub timeout: u32,
    pub sequence: u32,
    pub flags: u32,
    pub msg: [u8; MAX_MSG_SIZE],
    pub reply: u8,
    pub rx_status: u8,
    pub tx_status: u8,
    pub tx_arb_lost_cnt: u8,
    pub tx_nack_cnt: u8,
    pub tx_low_drive_cnt: u8,
    pub tx_error_cnt: u8,
}

impl Msg {
    /// A message with a header byte and an opcode, and nothing else.
    ///
    /// **The header is `(initiator << 4) | destination`** — one byte carrying
    /// both, which is the CEC frame and the reason a wrong nibble sends a
    /// television somebody else's instruction.
    #[must_use]
    pub fn command(initiator: u8, destination: u8, opcode: u8) -> Self {
        let mut msg = [0u8; MAX_MSG_SIZE];
        msg[0] = (initiator << 4) | (destination & 0x0f);
        msg[1] = opcode;
        Self {
            tx_ts: 0,
            rx_ts: 0,
            len: 2,
            // **Zero, and it means "do not wait for a reply".** `Standby` has
            // no reply defined, so a non-zero timeout here would block for
            // nothing and then report a timeout as a failure.
            timeout: 0,
            sequence: 0,
            flags: 0,
            msg,
            reply: 0,
            rx_status: 0,
            tx_status: 0,
            tx_arb_lost_cnt: 0,
            tx_nack_cnt: 0,
            tx_low_drive_cnt: 0,
            tx_error_cnt: 0,
        }
    }
}

/// `struct cec_caps` — 76 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Caps {
    pub driver: [u8; 32],
    pub name: [u8; 32],
    pub available_log_addrs: u32,
    pub capabilities: u32,
    pub version: u32,
}

impl Caps {
    pub const fn zeroed() -> Self {
        Self {
            driver: [0; 32],
            name: [0; 32],
            available_log_addrs: 0,
            capabilities: 0,
            version: 0,
        }
    }

    /// A NUL-terminated C string out of a fixed array, as text.
    fn text(field: &[u8]) -> String {
        let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
        String::from_utf8_lossy(&field[..end]).into_owned()
    }

    #[must_use]
    pub fn driver_name(&self) -> String {
        Self::text(&self.driver)
    }

    #[must_use]
    pub fn adapter_name(&self) -> String {
        Self::text(&self.name)
    }
}

/// `struct cec_log_addrs` — 92 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LogAddrs {
    pub log_addr: [u8; MAX_LOG_ADDRS],
    pub log_addr_mask: u16,
    pub cec_version: u8,
    pub num_log_addrs: u8,
    pub vendor_id: u32,
    pub flags: u32,
    pub osd_name: [u8; 15],
    pub primary_device_type: [u8; MAX_LOG_ADDRS],
    pub log_addr_type: [u8; MAX_LOG_ADDRS],
    pub all_device_types: [u8; MAX_LOG_ADDRS],
    pub features: [[u8; 12]; MAX_LOG_ADDRS],
}

impl LogAddrs {
    pub const fn zeroed() -> Self {
        Self {
            log_addr: [LOG_ADDR_UNREGISTERED; MAX_LOG_ADDRS],
            log_addr_mask: 0,
            cec_version: 0,
            num_log_addrs: 0,
            vendor_id: 0,
            flags: 0,
            osd_name: [0; 15],
            primary_device_type: [0; MAX_LOG_ADDRS],
            log_addr_type: [0; MAX_LOG_ADDRS],
            all_device_types: [0; MAX_LOG_ADDRS],
            features: [[0; 12]; MAX_LOG_ADDRS],
        }
    }

    /// **What this machine claims to be on the bus: a playback device.**
    ///
    /// Not a TV and not a tuner. A Rivet on the end of an HDMI cable is
    /// something you watch — the same class a set-top box or a games console
    /// claims — and the class decides which logical addresses the adapter is
    /// allowed to try for (4, 8 or 11).
    ///
    /// `osd_name` is what a television shows in its input list. Fifteen bytes,
    /// NUL-padded, and truncated rather than refused: a name too long is a
    /// cosmetic problem and refusing to configure over it would cost the whole
    /// feature.
    #[must_use]
    pub fn playback(osd: &str) -> Self {
        let mut addrs = Self::zeroed();
        addrs.cec_version = VERSION_2_0;
        addrs.num_log_addrs = 1;
        addrs.vendor_id = VENDOR_ID_NONE;
        addrs.primary_device_type[0] = PRIM_DEVTYPE_PLAYBACK;
        addrs.log_addr_type[0] = LOG_ADDR_TYPE_PLAYBACK;
        // `all_device_types` is the CEC 2.0 bitmap, and the playback bit is
        // `CEC_OP_ALL_DEVTYPE_PLAYBACK` = 0x04.
        addrs.all_device_types[0] = 0x04;
        for (slot, byte) in addrs.osd_name.iter_mut().zip(osd.bytes()) {
            *slot = byte;
        }
        addrs
    }

    /// The logical address the adapter actually claimed, if it claimed one.
    ///
    /// **Read back rather than assumed.** Which address a playback device gets
    /// depends on what else is already on the bus — 4, then 8, then 11 — so a
    /// program that hard-coded 4 would send its messages as whoever holds 4,
    /// which on a busy bus is somebody else's set-top box.
    #[must_use]
    pub fn claimed(&self) -> Option<u8> {
        (self.num_log_addrs > 0 && self.log_addr[0] != LOG_ADDR_UNREGISTERED)
            .then_some(self.log_addr[0])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use core::mem::{offset_of, size_of};

    /// **The sizes the header printed.** An ioctl request encodes the struct
    /// size, so a disagreement here is a request number the kernel refuses with
    /// `ENOTTY` — which reads as *unsupported device*, not as *wrong struct*.
    #[test]
    fn the_structs_are_the_size_the_kernel_header_says() {
        assert_eq!(size_of::<Msg>(), 56, "struct cec_msg");
        assert_eq!(size_of::<Caps>(), 76, "struct cec_caps");
        assert_eq!(size_of::<LogAddrs>(), 92, "struct cec_log_addrs");
    }

    /// And the size baked into each request number agrees with the struct it
    /// carries — `_IOC_SIZE` is bits 16..30.
    #[test]
    fn every_request_number_carries_the_size_of_its_own_struct() {
        let size_of_request = |request: u32| ((request >> 16) & 0x3fff) as usize;
        assert_eq!(size_of_request(ADAP_G_CAPS), size_of::<Caps>());
        assert_eq!(size_of_request(ADAP_G_LOG_ADDRS), size_of::<LogAddrs>());
        assert_eq!(size_of_request(ADAP_S_LOG_ADDRS), size_of::<LogAddrs>());
        assert_eq!(size_of_request(TRANSMIT), size_of::<Msg>());
        assert_eq!(size_of_request(ADAP_G_PHYS_ADDR), size_of::<u16>());
        assert_eq!(size_of_request(S_MODE), size_of::<u32>());
    }

    /// The offsets the header printed, field by field. A struct that is the
    /// right SIZE with a field in the wrong place passes the test above and
    /// still sends the kernel nonsense.
    #[test]
    fn every_field_is_where_the_kernel_header_puts_it() {
        assert_eq!(offset_of!(Msg, len), 16);
        assert_eq!(offset_of!(Msg, timeout), 20);
        assert_eq!(offset_of!(Msg, flags), 28);
        assert_eq!(offset_of!(Msg, msg), 32);
        assert_eq!(offset_of!(Msg, reply), 48);
        assert_eq!(offset_of!(Msg, tx_status), 50);

        assert_eq!(offset_of!(Caps, available_log_addrs), 64);
        assert_eq!(offset_of!(Caps, capabilities), 68);

        assert_eq!(offset_of!(LogAddrs, log_addr_mask), 4);
        assert_eq!(offset_of!(LogAddrs, cec_version), 6);
        assert_eq!(offset_of!(LogAddrs, num_log_addrs), 7);
        assert_eq!(offset_of!(LogAddrs, vendor_id), 8);
        assert_eq!(offset_of!(LogAddrs, flags), 12);
        assert_eq!(offset_of!(LogAddrs, osd_name), 16);
        assert_eq!(offset_of!(LogAddrs, primary_device_type), 31);
        assert_eq!(offset_of!(LogAddrs, log_addr_type), 35);
        assert_eq!(offset_of!(LogAddrs, all_device_types), 39);
        assert_eq!(offset_of!(LogAddrs, features), 43);
    }

    /// **The frame is one byte carrying two nibbles**, and getting them the
    /// wrong way round sends the television's own instruction back to it.
    #[test]
    fn a_command_frames_the_initiator_and_destination_into_one_byte() {
        // A playback device at logical address 4, telling the TV to sleep.
        let msg = Msg::command(4, LOG_ADDR_TV, MSG_STANDBY);
        assert_eq!(msg.len, 2);
        assert_eq!(msg.msg[0], 0x40, "initiator 4, destination 0");
        assert_eq!(msg.msg[1], 0x36);
        // Unregistered talking to the TV is 0xF0, which is a real frame and the
        // one an unconfigured adapter would send.
        let anon = Msg::command(LOG_ADDR_UNREGISTERED, LOG_ADDR_TV, MSG_IMAGE_VIEW_ON);
        assert_eq!(anon.msg[0], 0xf0);
        assert_eq!(anon.msg[1], 0x04);
        // A destination nibble is four bits and cannot leak into the initiator.
        assert_eq!(Msg::command(4, 0xff, MSG_STANDBY).msg[0], 0x4f);
    }

    /// A playback claim names itself as a playback device in all three of the
    /// places the kernel reads, and none of them may be left zero.
    #[test]
    fn a_playback_claim_is_consistent_across_the_three_fields_the_kernel_reads() {
        let addrs = LogAddrs::playback("Rivet");
        assert_eq!(addrs.num_log_addrs, 1);
        assert_eq!(addrs.primary_device_type[0], PRIM_DEVTYPE_PLAYBACK);
        assert_eq!(addrs.log_addr_type[0], LOG_ADDR_TYPE_PLAYBACK);
        assert_ne!(addrs.all_device_types[0], 0, "CEC 2.0 reads this one");
        assert_eq!(&addrs.osd_name[..5], b"Rivet");
        assert_eq!(addrs.osd_name[5], 0, "the name is NUL-padded");
        // Nothing is claimed until the kernel says so.
        assert_eq!(addrs.claimed(), None);
    }

    /// An OSD name longer than the field is truncated, not refused: a cosmetic
    /// problem must not cost the whole feature.
    #[test]
    fn an_over_long_screen_name_is_cut_rather_than_refused() {
        let addrs = LogAddrs::playback("a-very-long-machine-name-indeed");
        assert_eq!(addrs.osd_name.len(), 15);
        assert_eq!(&addrs.osd_name[..], b"a-very-long-mac");
    }

    /// The address the adapter claimed is read back, never assumed — which
    /// address a playback device gets depends on what else is on the bus.
    #[test]
    fn the_claimed_address_is_whatever_the_kernel_reports() {
        let mut addrs = LogAddrs::playback("Rivet");
        addrs.num_log_addrs = 1;
        addrs.log_addr[0] = 8;
        assert_eq!(addrs.claimed(), Some(8), "8 is what a second playback gets");
        addrs.log_addr[0] = LOG_ADDR_UNREGISTERED;
        assert_eq!(addrs.claimed(), None, "unregistered is not a claim");
    }
}
